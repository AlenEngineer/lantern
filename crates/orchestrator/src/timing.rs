//! How long things take, and whether a section is keeping to its budget.
//!
//! A step's length comes, in order, from the operator's
//! [estimate](crate::Step::estimate), the length its source gives
//! ([`Resource::DURATION`]), the length of its media once the app has loaded
//! it, and otherwise a guess by kind. A section is running from when its
//! first step was reached; its pace adds what's left of the current step
//! and the lengths of the steps after it, and compares that with the budget.
//! Nothing here stops the service: it only reports.

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, SystemTime};

use lantern_resource::{Kind, Resource};
use serde::{Deserialize, Serialize};

use crate::{Action, FlowOf, Plan, Repeat, Step, StepId};

/// Guesses for steps nobody gave a length.
const PER_SLIDE: Duration = Duration::from_secs(30);
const MEDIA: Duration = Duration::from_secs(4 * 60);
const DOCUMENT: Duration = Duration::from_secs(2 * 60);
const STILL: Duration = Duration::from_secs(30);
const LIVE_SLOT: Duration = Duration::from_secs(4 * 60);

/// How a step's length was found, so a guess can look like one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// The operator set it.
    Set,
    /// The source or the loaded media said so.
    Known,
    /// A guess by kind, or nothing to show (a note, a blank screen).
    Guess,
}

/// A step's expected length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Estimate {
    /// How long it should take.
    pub length: Duration,
    /// Where that came from.
    pub basis: Basis,
}

/// The length of `step`. `loaded` is its media's real length, if the app
/// has loaded it.
pub fn estimate(step: &Step, loaded: Option<Duration>) -> Estimate {
    let guess = |length| Estimate {
        length,
        basis: Basis::Guess,
    };
    if let Some(length) = step.estimate {
        return Estimate {
            length,
            basis: Basis::Set,
        };
    }
    match &step.action {
        Action::Show { resource } => media_length(resource, loaded),
        Action::Live { .. } => guess(LIVE_SLOT),
        Action::Blank | Action::Note | Action::Section { .. } => guess(Duration::ZERO),
    }
}

/// How long `resource` itself takes, whatever length a step showing it
/// is given: what its source says, its media's length once `loaded`, or
/// a guess by kind.
pub fn media_length(resource: &Resource, loaded: Option<Duration>) -> Estimate {
    match source_length(resource).or(loaded) {
        Some(length) => Estimate {
            length,
            basis: Basis::Known,
        },
        None => Estimate {
            length: guess_for(resource),
            basis: Basis::Guess,
        },
    }
}

fn source_length(resource: &Resource) -> Option<Duration> {
    let seconds = resource.meta.get(Resource::DURATION)?.parse().ok()?;
    Some(Duration::from_secs(seconds))
}

fn guess_for(resource: &Resource) -> Duration {
    match resource.kind {
        Kind::Text => {
            let slides = resource.location.as_text().map_or(1, |text| {
                text.split("\n\n").filter(|s| !s.trim().is_empty()).count()
            });
            PER_SLIDE * slides.max(1) as u32
        }
        Kind::Video | Kind::Audio => MEDIA,
        Kind::Document | Kind::Presentation => DOCUMENT,
        _ => STILL,
    }
}

/// A time of day to the minute, such as the 10:30 a service starts at.
/// Saved as it reads: `"10:30"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct TimeOfDay(u16);

impl TimeOfDay {
    /// Reads `10:30`, `9:05`, or `10` for ten o'clock, on a 24-hour clock.
    /// `None` for anything else.
    pub fn parse(text: &str) -> Option<TimeOfDay> {
        let (hour, minute) = match text.trim().split_once(':') {
            Some((hour, minute)) if minute.len() == 2 => (hour, minute.parse::<u16>().ok()?),
            Some(_) => return None,
            None => (text.trim(), 0),
        };
        let hour = hour.parse::<u16>().ok()?;
        (hour < 24 && minute < 60).then_some(TimeOfDay(hour * 60 + minute))
    }

    /// How long after midnight it is.
    pub fn since_midnight(self) -> Duration {
        Duration::from_secs(u64::from(self.0) * 60)
    }

    /// The time of day `after` later, to the minute below, past midnight
    /// if need be.
    pub fn plus(self, after: Duration) -> TimeOfDay {
        let minutes = (u64::from(self.0) + after.as_secs() / 60) % (24 * 60);
        TimeOfDay(minutes as u16)
    }
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{:02}", self.0 / 60, self.0 % 60)
    }
}

impl TryFrom<String> for TimeOfDay {
    type Error = String;
    fn try_from(text: String) -> Result<Self, String> {
        TimeOfDay::parse(&text).ok_or_else(|| format!("“{text}” isn't a time of day like 10:30"))
    }
}

impl From<TimeOfDay> for String {
    fn from(time: TimeOfDay) -> String {
        time.to_string()
    }
}

/// The plan as it was meant to go: how long after the start each section
/// begins, and how long the whole of it takes. A section counts for its
/// budget, or without one for its steps. A section a span cuts across,
/// taking in some of its steps and steps outside it, counts for its
/// steps: its time can't be told apart from the loop's.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Schedule {
    /// Each section's marker and how long after the start it begins.
    pub sections: Vec<(StepId, Duration)>,
    /// Each section's marker and how long after the start it ends, the
    /// first time round: for its budget, or without one its steps.
    pub ends: Vec<(StepId, Duration)>,
    /// Each step and how long after the start it first comes.
    pub steps: Vec<(StepId, Duration)>,
    /// From the start to the end. What repeats a number of times counts
    /// that many times.
    pub total: Duration,
    /// Something in the plan repeats with no number to it (until the
    /// operator goes elsewhere, or until a time): the total counts it
    /// once and the service may take longer.
    pub open: bool,
}

impl Schedule {
    /// How long after the start the section `id` begins.
    pub fn start_of(&self, id: StepId) -> Option<Duration> {
        self.sections
            .iter()
            .find(|(section, _)| *section == id)
            .map(|(_, after)| *after)
    }

    /// How long after the start the section `id` ends, the first time
    /// round.
    pub fn end_of(&self, id: StepId) -> Option<Duration> {
        self.ends
            .iter()
            .find(|(section, _)| *section == id)
            .map(|(_, after)| *after)
    }

    /// How long after the start `id` first comes: a section, or a step.
    pub fn first_at(&self, id: StepId) -> Option<Duration> {
        self.start_of(id).or_else(|| {
            self.steps
                .iter()
                .find(|(step, _)| *step == id)
                .map(|(_, after)| *after)
        })
    }
}

/// How something that comes again until a time of day comes out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lands {
    /// How long once round takes.
    pub round: Duration,
    /// When and how it ends. `None` in a plan with no start time, which
    /// can't say when it begins, and for something that takes no time.
    pub ends: Option<Ends>,
}

/// When and how something that comes again until a time of day ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ends {
    /// How many times round it goes, the last one counted though it may
    /// not be finished.
    pub rounds: u32,
    /// The time of day it ends, to the minute below.
    pub at: TimeOfDay,
    /// How long after the time asked for that is.
    pub over: Duration,
    /// The step that is cut off, and how far into it, when it ends
    /// there and then with a step on.
    pub cut: Option<(StepId, Duration)>,
}

/// How `of` (a step, a section, a span or the plan) comes out if it
/// comes again until `time` and ends as `ending` says. `loaded` is a
/// step's media length, once loaded. `None` if `of` isn't in the plan,
/// or is a span over no steps.
///
/// It goes round again while it is before the time. Then it finishes
/// the round ([`Ending::Round`]), or the step that is on
/// ([`Ending::Step`]), or stops there ([`Ending::Sharp`]). When it
/// begins is taken from the plan's start time and what comes before it,
/// each thing once round where it has no number of times. Its steps are
/// taken in the plan's order, each once.
pub fn lands(
    plan: &Plan,
    of: impl Into<FlowOf>,
    time: TimeOfDay,
    ending: crate::Ending,
    loaded: &dyn Fn(&Step) -> Option<Duration>,
) -> Option<Lands> {
    use crate::Ending;
    // Once round: the thing by itself, coming once. With it, where each
    // of its steps starts in a round.
    let once = |mut part: Plan| {
        part.flow.repeat = Repeat::Once;
        let schedule = schedule(&part, loaded);
        (schedule.total, schedule.steps)
    };
    let (after, round, steps) = match of.into() {
        FlowOf::Plan => {
            let (round, steps) = once(plan.clone());
            (Duration::ZERO, round, steps)
        }
        FlowOf::Span(id) => {
            let (first, _) = plan.span_range(id)?;
            let (round, steps) = once(plan.span_part(id)?);
            let first = plan.steps()[first].id;
            (schedule(plan, loaded).first_at(first)?, round, steps)
        }
        FlowOf::Step(id) => {
            let step = plan.get(id)?;
            let (round, steps) = match plan.part(id) {
                Some(part) => once(part),
                None => (
                    estimate(step, loaded(step)).length,
                    vec![(id, Duration::ZERO)],
                ),
            };
            (schedule(plan, loaded).first_at(id)?, round, steps)
        }
    };
    let ends = plan.starts.filter(|_| !round.is_zero()).map(|starts| {
        let begins = starts.since_midnight() + after;
        let until = time.since_midnight();
        // What is left of the time when it begins: none, when the time
        // has come already.
        let left = until.saturating_sub(begins);
        let whole = (left.as_secs_f64() / round.as_secs_f64()).floor() as u32;
        let into = left.saturating_sub(round * whole);
        let lands = |rounds: u32, took: Duration, cut| Ends {
            rounds: rounds.max(1),
            at: starts.plus(after + took),
            over: (begins + took).saturating_sub(until),
            cut,
        };
        // Ending right on the time, nothing is on to finish or cut. It
        // comes once at the least: begun after the time, it is the
        // first round that is on.
        if into.is_zero() && whole > 0 {
            return lands(whole, round * whole, None);
        }
        // The step that is on when the time comes, and when it ends.
        let on = steps
            .iter()
            .enumerate()
            .rev()
            .find(|(_, (_, start))| *start <= into)
            .map(|(at, (id, start))| {
                let end = steps.get(at + 1).map_or(round, |(_, next)| *next);
                (*id, *start, end)
            });
        match (ending, on) {
            (Ending::Sharp, Some((id, start, _))) => lands(
                whole + 1,
                round * whole + into,
                (left > Duration::ZERO).then_some((id, into - start)),
            ),
            (Ending::Step, Some((_, _, end))) => lands(whole + 1, round * whole + end, None),
            _ => lands(whole + 1, round * (whole + 1), None),
        }
    });
    Some(Lands { round, ends })
}

/// The schedule of `plan`. `loaded` is a step's media length, once loaded.
pub fn schedule(plan: &Plan, loaded: &dyn Fn(&Step) -> Option<Duration>) -> Schedule {
    // A join takes no time of its own, and may lie across the rest.
    let spans = plan
        .span_ranges()
        .into_iter()
        .filter_map(|(id, first, last)| {
            let flow = plan.span(id)?.flow;
            flow.shapes().then_some((first, last, flow.repeat))
        })
        .collect();
    let mut told = Told {
        plan,
        loaded,
        spans,
        next: 0,
        schedule: Schedule::default(),
    };
    let total = told.takes(0, plan.len(), Duration::ZERO);
    let mut schedule = told.schedule;
    // A section a span cuts across ends when what follows it begins.
    for (at, step) in plan.steps().iter().enumerate() {
        if !step.is_section() || schedule.end_of(step.id).is_some() {
            continue;
        }
        let end = plan.group_end(at);
        let after = plan.steps()[end..]
            .iter()
            .find_map(|next| schedule.first_at(next.id))
            .unwrap_or(total);
        schedule.ends.push((step.id, after));
    }
    schedule.open |= plan.flow.repeat.open();
    schedule.total = total * plan.flow.repeat.times();
    schedule
}

/// A schedule as it is told, row by row.
struct Told<'a> {
    plan: &'a Plan,
    loaded: &'a dyn Fn(&Step) -> Option<Duration>,
    /// Every span's first and last step and how often it comes,
    /// outermost first.
    spans: Vec<(usize, usize, Repeat)>,
    /// The first span not come to yet.
    next: usize,
    schedule: Schedule,
}

impl Told<'_> {
    /// How long the rows from `from` up to `to` take, starting `start`
    /// into the service: each step with its repeats, each span as many
    /// times as it comes, each section for its budget or without one for
    /// what it holds. Notes when each step and section first comes.
    fn takes(&mut self, from: usize, to: usize, start: Duration) -> Duration {
        let steps = self.plan.steps();
        let mut took = Duration::ZERO;
        let mut at = from;
        while at < to {
            let step = &steps[at];
            // A span that starts here: all of it, as often as it comes.
            let span = self.spans.get(self.next).copied();
            if let Some((_, last, repeat)) = span.filter(|(first, ..)| *first == at) {
                self.next += 1;
                self.schedule.open |= repeat.open();
                took += self.takes(at, last + 1, start + took) * repeat.times();
                at = last + 1;
            } else if let Action::Section { budget, .. } = step.action {
                self.schedule.sections.push((step.id, start + took));
                let end = self.plan.group_end(at);
                if self.cut_across(at, end) {
                    // Only a heading here: its steps count as they come.
                    at += 1;
                } else {
                    // A section's time is for all of it, however often
                    // it comes round.
                    let held = self.takes(at + 1, end, start + took);
                    took += budget.unwrap_or(held);
                    self.schedule.ends.push((step.id, start + took));
                    at = end;
                }
            } else {
                self.schedule.steps.push((step.id, start + took));
                self.schedule.open |= step.flow.repeat.open();
                let once = estimate(step, (self.loaded)(step)).length;
                took += once * step.flow.repeat.times();
                at += 1;
            }
        }
        took
    }

    /// Whether a span takes in some of the steps of the section at
    /// `at`, which holds the rows up to `end`, and steps outside it: one
    /// neither wholly in the section nor round all of it from before its
    /// heading.
    fn cut_across(&self, at: usize, end: usize) -> bool {
        let steps = self.plan.steps();
        let step = |row: &usize| !steps[*row].is_section();
        let mut held = at + 1..end;
        let Some((first, last)) = held.clone().find(step).zip(held.rfind(step)) else {
            return false;
        };
        self.spans.iter().any(|&(from, to, _)| {
            let apart = to < first || last < from;
            let within = first <= from && to <= last;
            let round = from < at && last <= to;
            !apart && !within && !round
        })
    }
}

/// How far behind its schedule a service runs (negative: ahead), in
/// seconds. `now` is the time of day as time since midnight, `ends_in` how
/// long until the service is expected to end, and the schedule says it
/// should end `total` after `starts`. A service around midnight reads
/// right: the difference is taken the short way round the clock.
pub fn behind(starts: TimeOfDay, total: Duration, now: Duration, ends_in: Duration) -> i64 {
    const DAY: i64 = 24 * 60 * 60;
    let scheduled_end = (starts.since_midnight() + total).as_secs() as i64;
    let expected_end = (now + ends_in).as_secs() as i64;
    (expected_end - scheduled_end + DAY / 2).rem_euclid(DAY) - DAY / 2
}

/// How a section is doing against its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    /// Expected to finish with time to spare.
    OnTrack,
    /// Expected to finish within the last minute or tenth of the budget.
    Tight,
    /// Past the budget, or expected to be.
    Over,
}

impl Pace {
    /// The pace of `expected` against `budget`: tight within a margin of a
    /// minute or a tenth of the budget, whichever is more.
    pub fn of(expected: Duration, budget: Duration) -> Pace {
        let margin = Duration::from_secs(60).max(budget / 10);
        if expected > budget {
            Pace::Over
        } else if expected + margin > budget {
            Pace::Tight
        } else {
            Pace::OnTrack
        }
    }
}

/// Where a section is in the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Not reached yet.
    Coming {
        /// The sum of its steps' lengths.
        planned: Duration,
    },
    /// Under way: `elapsed` since its first step was reached, and the
    /// `expected` total once the rest of it is done.
    Running {
        /// Since its first step was reached.
        elapsed: Duration,
        /// The total, once the rest of it is done.
        expected: Duration,
    },
    /// Over.
    Done {
        /// How long it took, when that's known.
        took: Option<Duration>,
    },
}

/// One section's timing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionTiming {
    /// The section's marker step.
    pub id: StepId,
    /// The marker's label.
    pub title: String,
    /// How long it should take, if the operator said.
    pub budget: Option<Duration>,
    /// Where it is in the service.
    pub phase: Phase,
    /// The time of day it should be over by, if the operator said.
    pub done_by: Option<TimeOfDay>,
    /// The time of day it is over, to the minute below: as planned from
    /// the plan's start time before the service runs, as expected from
    /// now while it runs, and once done, when it was. `None` where that
    /// can't be told: no start time before the service, or no time of
    /// day while it runs.
    pub ends: Option<TimeOfDay>,
}

impl SectionTiming {
    /// How it's doing against its budget: by what it's expected to take
    /// while it runs, by what it took once done. `None` before it starts or
    /// without a budget.
    pub fn pace(&self) -> Option<Pace> {
        let budget = self.budget?;
        match self.phase {
            Phase::Running { expected, .. } => Some(Pace::of(expected, budget)),
            Phase::Done { took } => took.map(|took| Pace::of(took, budget)),
            Phase::Coming { .. } => None,
        }
    }

    /// How far past the time it should be over by it ends, to the
    /// minute: `None` when it ends by then, or either time isn't known.
    /// The difference is taken the short way round the clock.
    pub fn late(&self) -> Option<Duration> {
        const DAY: i64 = 24 * 60 * 60;
        let (ends, by) = (self.ends?, self.done_by?);
        let secs = |time: TimeOfDay| time.since_midnight().as_secs() as i64;
        let past = (secs(ends) - secs(by) + DAY / 2).rem_euclid(DAY) - DAY / 2;
        (past > 0).then(|| Duration::from_secs(past as u64))
    }
}

/// The whole plan's timing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Timing {
    /// Every section, in order.
    pub sections: Vec<SectionTiming>,
    /// When the service is expected to end, once it has started. A section
    /// under way or still to come counts for its budget or its steps,
    /// whichever is more.
    pub end: Option<SystemTime>,
}

impl Timing {
    /// The section the service is in.
    pub fn running(&self) -> Option<&SectionTiming> {
        self.sections
            .iter()
            .find(|s| matches!(s.phase, Phase::Running { .. }))
    }
}

/// What the timing is computed from.
pub struct Clock<'a> {
    /// Now.
    pub now: SystemTime,
    /// The step the session is at.
    pub current: Option<StepId>,
    /// When a step was first reached.
    pub entered: &'a dyn Fn(StepId) -> Option<SystemTime>,
    /// What's left of the current step, when the stage knows exactly (a
    /// playing video). Otherwise its estimate less the time spent on it.
    pub current_left: Option<Duration>,
    /// A step's media length, once loaded.
    pub loaded: &'a dyn Fn(&Step) -> Option<Duration>,
    /// The time of day now, as time since midnight, when it is known:
    /// for saying at what time each section ends.
    pub of_day: Option<Duration>,
}

/// The time of day `since_midnight` is, past midnight if need be.
fn of_day(since_midnight: Duration) -> TimeOfDay {
    TimeOfDay(0).plus(since_midnight)
}

/// The time of day each section of `plan` is over, where that can be
/// told. Before the service it is as the [schedule] has it from the
/// plan's start time. While the service runs it is from now: what is
/// left of the step that is on, the steps after it, and each section
/// still to come for its budget, or for its steps where they take
/// longer; a section done says when it was.
fn section_ends(
    plan: &Plan,
    clock: &Clock<'_>,
    at: Option<usize>,
    current_left: Option<Duration>,
    length: &dyn Fn(&Step) -> Duration,
) -> HashMap<StepId, TimeOfDay> {
    let steps = plan.steps();
    let mut found = HashMap::new();
    let Some(at) = at else {
        if let Some(starts) = plan.starts {
            let schedule = schedule(plan, clock.loaded);
            for (id, after) in schedule.ends {
                found.insert(id, starts.plus(after));
            }
        }
        return found;
    };
    let Some(now) = clock.of_day else {
        return found;
    };
    let since = |time: SystemTime| clock.now.duration_since(time).unwrap_or_default();
    let day = Duration::from_secs(24 * 60 * 60);
    // Those done: when the first step after them was reached.
    for (start, step) in steps.iter().enumerate().take(at) {
        if !step.is_section() || plan.group_end(start) > at {
            continue;
        }
        let ended = steps[plan.group_end(start)..]
            .iter()
            .filter_map(|s| (clock.entered)(s.id))
            .min();
        if let Some(ended) = ended {
            found.insert(step.id, of_day(now + day - since(ended).min(day)));
        }
    }
    // Those under way, innermost first, then all that comes after.
    let mut after = HashMap::new();
    let mut from = at + 1;
    let mut took = current_left.unwrap_or_default();
    let within = plan.within(at);
    for id in within.iter().rev() {
        let Some(stop) = plan.position(*id).map(|start| plan.group_end(start)) else {
            continue;
        };
        took += ahead(plan, length, from, stop, took, &mut after);
        after.insert(*id, took);
        from = stop;
    }
    ahead(plan, length, from, steps.len(), took, &mut after);
    for (id, after) in after {
        found.insert(id, of_day(now + after));
    }
    found
}

/// How long the rows from `from` up to `to` take, from `start` on: each
/// step for its length, each section for its budget or for its steps
/// where they take longer, as often as the span attached to it has it
/// come. Notes in `ends` when each section ends, from the same start.
fn ahead(
    plan: &Plan,
    length: &dyn Fn(&Step) -> Duration,
    from: usize,
    to: usize,
    start: Duration,
    ends: &mut HashMap<StepId, Duration>,
) -> Duration {
    let steps = plan.steps();
    let mut took = Duration::ZERO;
    let mut at = from;
    while at < to {
        let step = &steps[at];
        if let Action::Section { budget, .. } = step.action {
            let stop = plan.group_end(at).min(to);
            let times = plan.flow_of(step.id).unwrap_or_default().repeat.times();
            let held = ahead(plan, length, at + 1, stop, start + took, ends) * times;
            took += budget.map_or(held, |budget| budget.max(held));
            ends.insert(step.id, start + took);
            at = stop.max(at + 1);
        } else {
            took += length(step);
            at += 1;
        }
    }
    took
}

/// The timing of every section of `plan`.
pub fn timing(plan: &Plan, clock: &Clock<'_>) -> Timing {
    let steps = plan.steps();
    // What repeats a number of times counts that many times.
    let length =
        |step: &Step| estimate(step, (clock.loaded)(step)).length * step.flow.repeat.times();
    let since = |time: SystemTime| clock.now.duration_since(time).unwrap_or_default();
    let at = clock.current.and_then(|id| plan.position(id));

    // What's left of the current step.
    let current_left = at.map(|i| {
        let step = &steps[i];
        clock.current_left.unwrap_or_else(|| {
            let spent = (clock.entered)(step.id).map(since).unwrap_or_default();
            length(step).saturating_sub(spent)
        })
    });

    let mut sections = Vec::new();
    let mut end = at.map(|_| clock.now + current_left.unwrap_or_default());
    // Steps after the current one at the plan's own level, in no
    // section, count too.
    if let Some(i) = at {
        let rest: Duration = (i + 1..steps.len())
            .filter(|&row| !steps[row].is_section() && plan.level(row) == 0)
            .map(|row| length(&steps[row]))
            .sum();
        end = end.map(|end| end + rest);
    }
    let ends = section_ends(plan, clock, at, current_left, &length);

    let markers = steps
        .iter()
        .enumerate()
        .filter_map(|(i, step)| match step.action {
            Action::Section { budget, depth, .. } => Some((i, step, budget, depth)),
            _ => None,
        });
    for (start, step, budget, depth) in markers {
        // A section holds the sections in it: their steps are its steps.
        // Only the plan's own sections add to when the service ends, or
        // a section in another would be counted twice.
        let outer = depth == 0;
        let stop = plan.group_end(start);
        let members = &steps[start + 1..stop];
        // As often as the span attached to it has it come.
        let times = plan.flow_of(step.id).unwrap_or_default().repeat.times();
        let planned: Duration = members.iter().map(length).sum::<Duration>() * times;
        let began = members.iter().filter_map(|s| (clock.entered)(s.id)).min();
        let phase = match at {
            Some(i) if (start + 1..stop).contains(&i) => {
                let elapsed = began.map(since).unwrap_or_default();
                let after: Duration = steps[i + 1..stop].iter().map(length).sum();
                let left = current_left.unwrap_or_default() + after;
                // Like one still to come, it counts for its budget when
                // its steps would end sooner: the time is its to use.
                let spare = budget.map_or(Duration::ZERO, |b| b.saturating_sub(elapsed + left));
                if outer {
                    end = end.map(|end| end + after + spare);
                }
                Phase::Running {
                    elapsed,
                    expected: elapsed + left,
                }
            }
            Some(i) if i >= stop => {
                let ended = steps[stop..]
                    .iter()
                    .filter_map(|s| (clock.entered)(s.id))
                    .min();
                let took = began
                    .zip(ended)
                    .map(|(began, ended)| ended.duration_since(began).unwrap_or_default());
                Phase::Done { took }
            }
            _ => {
                if outer {
                    end = end.map(|end| end + budget.map_or(planned, |b| b.max(planned)));
                }
                Phase::Coming { planned }
            }
        };
        sections.push(SectionTiming {
            id: step.id,
            title: step.label.clone(),
            budget,
            phase,
            done_by: step.done_by(),
            ends: ends.get(&step.id).copied(),
        });
    }
    Timing { sections, end }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use lantern_resource::{Location, ResourceId, SourceId};

    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn video(title: &str, seconds: Option<u64>) -> Resource {
        let source = SourceId::new("test").unwrap();
        let resource = Resource::new(
            ResourceId::new(&source, title),
            title,
            Kind::Video,
            Location::Url(title.into()),
        );
        match seconds {
            Some(s) => resource.with_meta(Resource::DURATION, s.to_string()),
            None => resource,
        }
    }

    fn verses(title: &str, slides: usize) -> Resource {
        let source = SourceId::new("test").unwrap();
        let body = vec!["verse"; slides].join("\n\n");
        Resource::new(
            ResourceId::new(&source, title),
            title,
            Kind::Text,
            Location::Inline(body),
        )
    }

    /// Alabanza (15 min): three 4-minute songs. Palabra (30 min): a sermon.
    fn service() -> (Plan, Vec<StepId>) {
        let mut plan = Plan::new("Sunday");
        let worship = plan.section("Alabanza", Some(15 * MIN));
        let a = plan.show(video("A", Some(240)));
        let b = plan.show(video("B", Some(240)));
        let c = plan.show(video("C", Some(240)));
        let word = plan.section("Palabra", Some(30 * MIN));
        let sermon = plan.push("Sermon", Action::Blank);
        (plan, vec![worship, a, b, c, word, sermon])
    }

    #[test]
    fn a_time_of_day_reads_and_writes_as_people_do() {
        let half_ten = TimeOfDay::parse("10:30").unwrap();
        assert_eq!(half_ten.to_string(), "10:30");
        assert_eq!(TimeOfDay::parse(" 9:05 ").unwrap().to_string(), "9:05");
        assert_eq!(TimeOfDay::parse("19").unwrap().to_string(), "19:00");
        for bad in ["", "10:", "10:3", "24:00", "10:60", "ten", "1030"] {
            assert_eq!(TimeOfDay::parse(bad), None, "{bad}");
        }
        assert_eq!(half_ten.plus(50 * MIN).to_string(), "11:20");
        assert_eq!(
            TimeOfDay::parse("23:50")
                .unwrap()
                .plus(20 * MIN)
                .to_string(),
            "0:10"
        );
        let json = serde_json::to_string(&half_ten).unwrap();
        assert_eq!(json, "\"10:30\"");
        assert_eq!(serde_json::from_str::<TimeOfDay>(&json).unwrap(), half_ten);
        assert!(serde_json::from_str::<TimeOfDay>("\"late\"").is_err());
    }

    #[test]
    fn the_schedule_counts_a_section_for_its_budget_or_else_its_steps() {
        let (mut plan, ids) = service();
        // A welcome before any section, and a section without a budget.
        plan.insert(
            0,
            "Welcome",
            Action::Show {
                resource: video("W", Some(120)),
            },
        );
        plan.section("Cierre", None);
        plan.show(video("Z", Some(180)));
        let close = plan.steps()[plan.len() - 2].id;
        let schedule = schedule(&plan, &|_| None);
        assert_eq!(schedule.start_of(ids[0]), Some(2 * MIN));
        // Alabanza counts for its 15 minutes, not the 12 its songs add up to.
        assert_eq!(schedule.start_of(ids[4]), Some(17 * MIN));
        assert_eq!(schedule.start_of(close), Some(47 * MIN));
        assert_eq!(schedule.total, 50 * MIN);
        assert_eq!(schedule.start_of(ids[1]), None, "a step isn't a section");
    }

    #[test]
    fn behind_is_the_expected_end_against_the_scheduled_one() {
        let starts = TimeOfDay::parse("10:30").unwrap();
        let of_day = |text: &str| TimeOfDay::parse(text).unwrap().since_midnight();
        // Should end at 11:20. At 10:50 with 35 minutes to go: 5 behind.
        assert_eq!(behind(starts, 50 * MIN, of_day("10:50"), 35 * MIN), 300);
        assert_eq!(behind(starts, 50 * MIN, of_day("10:50"), 30 * MIN), 0);
        assert_eq!(behind(starts, 50 * MIN, of_day("10:50"), 28 * MIN), -120);
        // A vigil over midnight: should end at 0:20, expected at 0:25.
        let late = TimeOfDay::parse("23:30").unwrap();
        assert_eq!(behind(late, 50 * MIN, of_day("23:55"), 30 * MIN), 300);
        assert_eq!(behind(late, 50 * MIN, of_day("0:05"), 10 * MIN), -300);
    }

    fn at(
        plan: &Plan,
        now: SystemTime,
        current: Option<StepId>,
        entered: &HashMap<StepId, SystemTime>,
    ) -> Timing {
        let entered = |id| entered.get(&id).copied();
        let loaded = |_: &Step| None;
        timing(
            plan,
            &Clock {
                now,
                current,
                entered: &entered,
                current_left: None,
                loaded: &loaded,
                of_day: None,
            },
        )
    }

    #[test]
    fn lengths_come_from_the_operator_the_source_the_media_then_a_guess() {
        let (mut plan, ids) = service();
        let step = |plan: &Plan, i: usize| plan.get(ids[i]).unwrap().clone();
        assert_eq!(estimate(&step(&plan, 1), None).length, 4 * MIN);
        assert_eq!(estimate(&step(&plan, 1), None).basis, Basis::Known);
        plan.get_mut(ids[1]).unwrap().estimate = Some(5 * MIN);
        assert_eq!(estimate(&step(&plan, 1), None).basis, Basis::Set);
        let Action::Show { resource } = &step(&plan, 1).action else {
            panic!("a video step")
        };
        assert_eq!(
            media_length(resource, None),
            Estimate {
                length: 4 * MIN,
                basis: Basis::Known
            },
            "what the video takes stays its own"
        );

        let mut unknown = Plan::new("x");
        let id = unknown.show(video("D", None));
        let d = unknown.get(id).unwrap();
        assert_eq!(
            estimate(d, None),
            Estimate {
                length: MEDIA,
                basis: Basis::Guess
            }
        );
        assert_eq!(estimate(d, Some(MIN)).basis, Basis::Known);

        let id = unknown.show(verses("Juan 3:16-18", 3));
        assert_eq!(
            estimate(unknown.get(id).unwrap(), None).length,
            3 * PER_SLIDE
        );
    }

    #[test]
    fn before_the_start_sections_are_coming_and_there_is_no_end() {
        let (plan, _) = service();
        let timing = at(&plan, SystemTime::now(), None, &HashMap::new());
        assert_eq!(
            timing.sections[0].phase,
            Phase::Coming { planned: 12 * MIN }
        );
        assert_eq!(timing.sections[0].pace(), None);
        assert_eq!(timing.end, None);
    }

    #[test]
    fn a_running_section_adds_what_is_left_to_what_has_passed() {
        let (plan, ids) = service();
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        // Song A took 6 minutes; song B started 1 minute ago.
        let entered = HashMap::from([(ids[1], start), (ids[2], start + 6 * MIN)]);
        let now = start + 7 * MIN;
        let timing = at(&plan, now, Some(ids[2]), &entered);
        let worship = &timing.sections[0];
        // 7 elapsed + 3 left of B + 4 of C = 14 of 15: within the margin.
        assert_eq!(
            worship.phase,
            Phase::Running {
                elapsed: 7 * MIN,
                expected: 14 * MIN
            }
        );
        assert_eq!(worship.pace(), Some(Pace::Tight));
        assert_eq!(timing.running().map(|s| s.title.as_str()), Some("Alabanza"));
        // The minute of its budget the songs leave over, then the sermon's
        // 30-minute budget, more than its steps.
        assert_eq!(timing.end, Some(now + 7 * MIN + MIN + 30 * MIN));
        // So a service that started 6 minutes late, and has kept to its
        // times since, is 6 minutes behind: not ahead by the spare minute.
        let starts = TimeOfDay::parse("10:00").unwrap();
        let schedule = schedule(&plan, &|_| None);
        let of_day = TimeOfDay::parse("10:13").unwrap().since_midnight();
        let ends_in = timing.end.unwrap().duration_since(now).unwrap();
        assert_eq!(behind(starts, schedule.total, of_day, ends_in), 6 * 60);
    }

    /// The timing at `now`, which is `of_day` by the clock.
    fn at_day(
        plan: &Plan,
        now: SystemTime,
        current: Option<StepId>,
        entered: &HashMap<StepId, SystemTime>,
        of_day: &str,
    ) -> Timing {
        let entered = |id| entered.get(&id).copied();
        let loaded = |_: &Step| None;
        timing(
            plan,
            &Clock {
                now,
                current,
                entered: &entered,
                current_left: None,
                loaded: &loaded,
                of_day: TimeOfDay::parse(of_day).map(TimeOfDay::since_midnight),
            },
        )
    }

    #[test]
    fn a_section_says_when_it_ends_and_how_far_past_its_time_that_is() {
        let (mut plan, ids) = service();
        let time = |text: &str| TimeOfDay::parse(text);
        for (section, by) in [(ids[0], "10:12"), (ids[4], "11:00")] {
            if let Action::Section { done_by, .. } = &mut plan.get_mut(section).unwrap().action {
                *done_by = time(by);
            }
        }
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let none = HashMap::new();
        // No start time, nothing running: no time it ends.
        let timing = at(&plan, start, None, &none);
        assert_eq!(timing.sections[0].done_by, time("10:12"));
        assert_eq!(timing.sections[0].ends, None);
        assert_eq!(timing.sections[0].late(), None);
        // From the start time, by the schedule: Alabanza for its 15
        // minutes, three past its time; Palabra in time.
        plan.starts = time("10:00");
        assert_eq!(schedule(&plan, &|_| None).end_of(ids[0]), Some(15 * MIN));
        let timing = at(&plan, start, None, &none);
        assert_eq!(timing.sections[0].ends, time("10:15"));
        assert_eq!(timing.sections[0].late(), Some(3 * MIN));
        assert_eq!(timing.sections[1].ends, time("10:45"));
        assert_eq!(timing.sections[1].late(), None);
        // Running, from now: B started a minute ago at 10:13, so 3 left
        // of it and C's 4; then Palabra's 30.
        let now = start + 7 * MIN;
        let entered = HashMap::from([(ids[1], start), (ids[2], start + 6 * MIN)]);
        let timing = at_day(&plan, now, Some(ids[2]), &entered, "10:13");
        assert_eq!(timing.sections[0].ends, time("10:20"));
        assert_eq!(timing.sections[0].late(), Some(8 * MIN));
        assert_eq!(timing.sections[1].ends, time("10:50"));
        // Without the time of day while it runs, it can't be told.
        let timing = at(&plan, now, Some(ids[2]), &entered);
        assert_eq!(timing.sections[0].ends, None);
        // Done: when the step after it was reached, four minutes ago.
        let entered = HashMap::from([(ids[1], start), (ids[5], start + 16 * MIN)]);
        let timing = at_day(&plan, start + 20 * MIN, Some(ids[5]), &entered, "10:20");
        assert_eq!(timing.sections[0].ends, time("10:16"));
        assert_eq!(timing.sections[0].late(), Some(4 * MIN));
    }

    #[test]
    fn a_step_after_a_sections_own_end_counts_toward_the_end_of_the_service() {
        let (mut plan, ids) = service();
        // Alabanza ends at B: C is the plan's own, after it.
        plan.reach_section(ids[0], Some(ids[2])).unwrap();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let entered = HashMap::from([(ids[1], now)]);
        let timing = at(&plan, now, Some(ids[1]), &entered);
        // A's 4 and B's, the 7 Alabanza leaves over, C's 4, Palabra's 30.
        assert_eq!(
            timing.end,
            Some(now + 4 * MIN + 4 * MIN + 7 * MIN + 4 * MIN + 30 * MIN)
        );
        let timing = at_day(&plan, now, Some(ids[1]), &entered, "10:00");
        assert_eq!(timing.sections[0].ends, TimeOfDay::parse("10:08"));
        assert_eq!(timing.sections[1].ends, TimeOfDay::parse("10:42"));
    }

    #[test]
    fn a_done_section_reports_what_it_took() {
        let (plan, ids) = service();
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let entered = HashMap::from([(ids[1], start), (ids[5], start + 16 * MIN)]);
        let timing = at(&plan, start + 20 * MIN, Some(ids[5]), &entered);
        assert_eq!(
            timing.sections[0].phase,
            Phase::Done {
                took: Some(16 * MIN)
            }
        );
        assert_eq!(timing.sections[0].pace(), Some(Pace::Over));
        assert_eq!(timing.running().map(|s| s.title.as_str()), Some("Palabra"));
    }

    #[test]
    fn pace_has_a_margin_of_a_minute_or_a_tenth() {
        assert_eq!(Pace::of(10 * MIN, 15 * MIN), Pace::OnTrack);
        assert_eq!(Pace::of(14 * MIN, 15 * MIN), Pace::Tight);
        assert_eq!(Pace::of(16 * MIN, 15 * MIN), Pace::Over);
        assert_eq!(Pace::of(26 * MIN, 30 * MIN), Pace::OnTrack);
        assert_eq!(
            Pace::of(28 * MIN, 30 * MIN),
            Pace::Tight,
            "a tenth of 30 is 3"
        );
    }

    #[test]
    fn a_section_in_another_counts_once() {
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        let praise = plan.section("Praise", Some(10 * MIN));
        plan.show(video("A", Some(120)));
        let reading = plan.section("Reading", None);
        plan.show(video("B", Some(60)));
        let sermon = plan.section("Sermon", None);
        plan.show(video("C", Some(300)));
        assert!(plan.nest(praise, true));
        assert!(plan.nest(reading, true));

        // Music has no budget: Praise for its 10 min, Reading for its step.
        let schedule = schedule(&plan, &|_| None);
        assert_eq!(schedule.start_of(music), Some(Duration::ZERO));
        assert_eq!(schedule.start_of(praise), Some(Duration::ZERO));
        assert_eq!(schedule.start_of(reading), Some(10 * MIN));
        assert_eq!(schedule.start_of(sermon), Some(11 * MIN));
        assert_eq!(schedule.total, 16 * MIN);

        // Before the service, Music says what all its steps add up to.
        let entered = |_| None;
        let loaded = |_: &Step| None;
        let clock = Clock {
            now: SystemTime::UNIX_EPOCH,
            current: None,
            entered: &entered,
            current_left: None,
            loaded: &loaded,
            of_day: None,
        };
        let timing = timing(&plan, &clock);
        let planned = |id| match timing.sections.iter().find(|s| s.id == id).unwrap().phase {
            Phase::Coming { planned } => planned,
            other => panic!("{other:?}"),
        };
        assert_eq!(planned(music), 3 * MIN);
        assert_eq!(planned(praise), 2 * MIN);
        assert_eq!(planned(reading), MIN);
    }

    #[test]
    fn until_a_time_says_how_many_rounds_and_how_far_past_it_ends() {
        use crate::Ending;
        let mut plan = Plan::new("Sabbath");
        let welcome = plan.show(video("Welcome", Some(240)));
        let music = plan.section("Music", None);
        let a = plan.show(video("A", Some(180)));
        let b = plan.show(video("B", Some(120)));
        let at = |text: &str| TimeOfDay::parse(text).unwrap();
        let ends = |plan: &Plan, of, time: &str, ending| {
            lands(plan, of, at(time), ending, &|_| None).unwrap().ends
        };
        let came = |rounds, time: &str, over, cut| {
            Some(Ends {
                rounds,
                at: at(time),
                over,
                cut,
            })
        };
        // With no start time: only how long once round takes.
        let got = lands(&plan, Some(music), at("10:30"), Ending::Round, &|_| None).unwrap();
        assert_eq!((got.round, got.ends), (5 * MIN, None));
        // Starting 10:00, Music begins 10:04 and takes 5 min a round: A
        // from :04, B from :07. 10:30 comes a minute into the sixth A.
        plan.starts = Some(at("10:00"));
        // The round is finished: 10:34.
        assert_eq!(
            ends(&plan, Some(music), "10:30", Ending::Round),
            came(6, "10:34", 4 * MIN, None)
        );
        // The step is finished: A ends 10:32, and B is left out.
        assert_eq!(
            ends(&plan, Some(music), "10:30", Ending::Step),
            came(6, "10:32", 2 * MIN, None)
        );
        // There and then: A is cut a minute in.
        assert_eq!(
            ends(&plan, Some(music), "10:30", Ending::Sharp),
            came(6, "10:30", Duration::ZERO, Some((a, MIN)))
        );
        // Into B: its end is the round's.
        assert_eq!(
            ends(&plan, Some(music), "10:33", Ending::Step),
            came(6, "10:34", MIN, None)
        );
        assert_eq!(
            ends(&plan, Some(music), "10:33", Ending::Sharp),
            came(6, "10:33", Duration::ZERO, Some((b, MIN)))
        );
        // Ending right on the time is not gone round again, however it ends.
        for ending in [Ending::Round, Ending::Step, Ending::Sharp] {
            assert_eq!(
                ends(&plan, Some(music), "10:29", ending),
                came(5, "10:29", Duration::ZERO, None),
                "{ending:?}"
            );
        }
        // A time already past when it begins: once round, or its first
        // step, or nothing of it.
        assert_eq!(
            ends(&plan, Some(music), "10:02", Ending::Round),
            came(1, "10:09", 7 * MIN, None)
        );
        assert_eq!(
            ends(&plan, Some(music), "10:02", Ending::Step),
            came(1, "10:07", 5 * MIN, None)
        );
        assert_eq!(
            ends(&plan, Some(music), "10:02", Ending::Sharp),
            came(1, "10:04", 2 * MIN, None)
        );
        // A step by itself: A begins 10:04, three minutes a time.
        assert_eq!(
            ends(&plan, Some(a), "10:12", Ending::Round),
            came(3, "10:13", MIN, None)
        );
        // The plan: nine minutes round, from 10:00.
        assert_eq!(
            ends(&plan, None, "10:20", Ending::Round),
            came(3, "10:27", 7 * MIN, None)
        );
        // What is before it counts where it sits.
        assert_eq!(
            schedule(&plan, &|_| None).first_at(welcome),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn what_repeats_counts_as_often_as_it_comes() {
        use crate::{Flow, Repeat};
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        let a = plan.show(video("A", Some(120)));
        plan.show(video("B", Some(60)));
        let sermon = plan.section("Sermon", Some(20 * MIN));
        plan.show(video("C", Some(300)));
        // A twice each time, Music three times round: (2+2+1) x 3.
        let times = |times| Flow {
            repeat: Repeat::Times(times),
            ..Flow::default()
        };
        plan.get_mut(a).unwrap().flow.repeat = Repeat::Times(2);
        plan.set_flow(music, times(3)).unwrap();
        // A section's time is for all of it, repeats and all.
        plan.set_flow(sermon, times(2)).unwrap();
        let got = schedule(&plan, &|_| None);
        assert_eq!(got.start_of(sermon), Some(15 * MIN));
        assert_eq!(got.total, 35 * MIN);
        assert!(!got.open);
        // Until you go elsewhere has no number: counted once, and said.
        let forever = Flow {
            repeat: Repeat::Forever,
            ..Flow::default()
        };
        plan.set_flow(music, forever).unwrap();
        let got = schedule(&plan, &|_| None);
        assert_eq!(got.total, 25 * MIN);
        assert!(got.open);
        // Before the service, Music says its steps as often as it comes.
        plan.set_flow(music, times(3)).unwrap();
        let entered = |_| None;
        let loaded = |_: &Step| None;
        let clock = Clock {
            now: SystemTime::UNIX_EPOCH,
            current: None,
            entered: &entered,
            current_left: None,
            loaded: &loaded,
            of_day: None,
        };
        assert_eq!(
            timing(&plan, &clock).sections[0].phase,
            Phase::Coming { planned: 15 * MIN }
        );
    }

    /// Music: A (2 min), B (1 min). Sermon (20 min): C (5 min), D (3 min).
    fn two_parts() -> (Plan, [StepId; 6]) {
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        let a = plan.show(video("A", Some(120)));
        let b = plan.show(video("B", Some(60)));
        let sermon = plan.section("Sermon", Some(20 * MIN));
        let c = plan.show(video("C", Some(300)));
        let d = plan.show(video("D", Some(180)));
        (plan, [music, a, b, sermon, c, d])
    }

    #[test]
    fn a_loop_across_a_heading_counts_its_steps_as_often_as_they_come() {
        use crate::{Flow, Repeat};
        let (mut plan, [music, a, b, sermon, c, d]) = two_parts();
        // B and C three times round: Sermon is cut across, so it counts
        // for its steps and not its 20 minutes.
        let looped = plan.loop_over(b, c).unwrap();
        let thrice = Flow {
            repeat: Repeat::Times(3),
            ..Flow::default()
        };
        plan.set_flow(looped, thrice).unwrap();
        let got = schedule(&plan, &|_| None);
        // A 2, then (B 1 + C 5) x 3, then D 3.
        assert_eq!(got.total, 23 * MIN);
        assert_eq!(got.start_of(music), Some(Duration::ZERO));
        assert_eq!(got.first_at(b), Some(2 * MIN));
        // Sermon's heading is among the loop's rows: it first comes
        // after B, the first time round.
        assert_eq!(got.start_of(sermon), Some(3 * MIN));
        assert_eq!(got.first_at(c), Some(3 * MIN));
        assert_eq!(got.first_at(d), Some(20 * MIN));
        // A loop round all of Sermon and more leaves Sermon whole: its
        // 20 minutes, each time round.
        plan.remove_span(looped);
        plan.loop_over(a, d).unwrap();
        let got = schedule(&plan, &|_| None);
        assert_eq!(got.total, (3 + 20) * 2 * MIN);
        assert_eq!(got.start_of(sermon), Some(3 * MIN));
        // And one wholly in Sermon is inside its 20 minutes.
        let (mut plan, [.., c, d]) = two_parts();
        plan.loop_over(c, d).unwrap();
        assert_eq!(schedule(&plan, &|_| None).total, 23 * MIN);
    }

    #[test]
    fn a_span_until_a_time_says_how_it_comes_out_as_a_section_does() {
        use crate::Ending;
        let (mut plan, [_, a, b, _, c, _]) = two_parts();
        let at = |text: &str| TimeOfDay::parse(text).unwrap();
        plan.starts = Some(at("10:00"));
        // B and C, across the heading: it begins 10:02 after A, and once
        // round takes 6 minutes, not Sermon's 20: it holds only C of it.
        // 10:17 comes two minutes into the third C.
        let looped = plan.loop_over(b, c).unwrap();
        let got = lands(&plan, looped, at("10:17"), Ending::Sharp, &|_| None).unwrap();
        assert_eq!(got.round, 6 * MIN);
        assert_eq!(
            got.ends,
            Some(Ends {
                rounds: 3,
                at: at("10:17"),
                over: Duration::ZERO,
                cut: Some((c, 2 * MIN)),
            })
        );
        // A span in it counts in its round: A to C, with B and C twice.
        let outer = plan.loop_over(a, c).unwrap();
        let got = lands(&plan, outer, at("10:15"), Ending::Round, &|_| None).unwrap();
        assert_eq!(got.round, (2 + 6 * 2) * MIN);
        assert_eq!(got.ends.map(|ends| ends.rounds), Some(2));
    }
}
