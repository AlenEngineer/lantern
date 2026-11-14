//! How long things take, and whether a section is keeping to its budget.
//!
//! A step's length comes, in order, from the operator's
//! [estimate](crate::Step::estimate), the length its source gives
//! ([`Resource::DURATION`]), the length of its media once the app has loaded
//! it, and otherwise a guess by kind. A section is running from when its
//! first step was reached; its pace adds what's left of the current step
//! and the lengths of the steps after it, and compares that with the budget.
//! Nothing here stops the service: it only reports.

use std::fmt;
use std::time::{Duration, SystemTime};

use lantern_resource::{Kind, Resource};
use serde::{Deserialize, Serialize};

use crate::{Action, Plan, Step, StepId};

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
    let known = |length| Estimate {
        length,
        basis: Basis::Known,
    };
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
        Action::Show { resource } => match source_length(resource).or(loaded) {
            Some(length) => known(length),
            None => guess(guess_for(resource)),
        },
        Action::Live { .. } => guess(LIVE_SLOT),
        Action::Blank | Action::Note | Action::Section { .. } => guess(Duration::ZERO),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
/// budget, or without one for its steps.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Schedule {
    /// Each section's marker and how long after the start it begins.
    pub sections: Vec<(StepId, Duration)>,
    /// From the start to the end.
    pub total: Duration,
}

impl Schedule {
    /// How long after the start the section `id` begins.
    pub fn start_of(&self, id: StepId) -> Option<Duration> {
        self.sections
            .iter()
            .find(|(section, _)| *section == id)
            .map(|(_, after)| *after)
    }
}

/// The schedule of `plan`. `loaded` is a step's media length, once loaded.
pub fn schedule(plan: &Plan, loaded: &dyn Fn(&Step) -> Option<Duration>) -> Schedule {
    let mut schedule = Schedule::default();
    // What the section under way adds up to, and its budget.
    let mut planned = Duration::ZERO;
    let mut budget = None;
    for step in plan.steps() {
        match step.action {
            Action::Section { budget: next } => {
                schedule.total += budget.unwrap_or(planned);
                schedule.sections.push((step.id, schedule.total));
                planned = Duration::ZERO;
                budget = next;
            }
            _ => planned += estimate(step, loaded(step)).length,
        }
    }
    schedule.total += budget.unwrap_or(planned);
    schedule
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
}

/// The timing of every section of `plan`.
pub fn timing(plan: &Plan, clock: &Clock<'_>) -> Timing {
    let steps = plan.steps();
    let length = |step: &Step| estimate(step, (clock.loaded)(step)).length;
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
    // Steps after the current one but before any section count too.
    let first_section = steps
        .iter()
        .position(Step::is_section)
        .unwrap_or(steps.len());
    if let Some(i) = at.filter(|&i| i < first_section) {
        let rest: Duration = steps[i + 1..first_section].iter().map(length).sum();
        end = end.map(|end| end + rest);
    }

    let markers = steps
        .iter()
        .enumerate()
        .filter_map(|(i, step)| match step.action {
            Action::Section { budget } => Some((i, step, budget)),
            _ => None,
        });
    for (start, step, budget) in markers {
        let stop = steps[start + 1..]
            .iter()
            .position(Step::is_section)
            .map_or(steps.len(), |n| start + 1 + n);
        let members = &steps[start + 1..stop];
        let planned: Duration = members.iter().map(length).sum();
        let began = members.iter().filter_map(|s| (clock.entered)(s.id)).min();
        let phase = match at {
            Some(i) if (start + 1..stop).contains(&i) => {
                let elapsed = began.map(since).unwrap_or_default();
                let after: Duration = steps[i + 1..stop].iter().map(length).sum();
                let left = current_left.unwrap_or_default() + after;
                // Like one still to come, it counts for its budget when
                // its steps would end sooner: the time is its to use.
                let spare = budget.map_or(Duration::ZERO, |b| b.saturating_sub(elapsed + left));
                end = end.map(|end| end + after + spare);
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
                end = end.map(|end| end + budget.map_or(planned, |b| b.max(planned)));
                Phase::Coming { planned }
            }
        };
        sections.push(SectionTiming {
            id: step.id,
            title: step.label.clone(),
            budget,
            phase,
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
}
