//! How long things take, and whether a section is keeping to its budget.
//!
//! A step's length comes, in order, from the operator's
//! [estimate](crate::Step::estimate), the length its source gives
//! ([`Resource::DURATION`]), the length of its media once the app has loaded
//! it, and otherwise a guess by kind. A section is running from when its
//! first step was reached; its pace adds what's left of the current step
//! and the lengths of the steps after it, and compares that with the budget.
//! Nothing here stops the service: it only reports.

use std::time::{Duration, SystemTime};

use lantern_resource::{Kind, Resource};

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
    /// still to come counts for its budget or its steps, whichever is more.
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
                end = end.map(|end| end + after);
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
        // Then the sermon's 30-minute budget, more than its steps.
        assert_eq!(timing.end, Some(now + 7 * MIN + 30 * MIN));
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
