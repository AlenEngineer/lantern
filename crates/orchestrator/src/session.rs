use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use lantern_resource::Resource;
use serde::{Deserialize, Serialize};

use crate::{Action, Flow, Plan, Repeat, Step, StepId};

/// Something that was put on stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    /// When.
    pub at: SystemTime,
    /// The step the session was at, if it had started.
    pub step: Option<StepId>,
    /// What was shown.
    pub resource: Resource,
    /// `true` if it came from a [`Action::Show`] step, `false` if it was shown on the fly.
    pub planned: bool,
}

/// A plan being run: where we are, and what has been shown.
///
/// The cursor points at a step *id*, not an index, so the plan can be edited
/// mid-session (add a song the pastor just asked for) without losing your place.
/// It never rests on a section marker: stepping passes over them.
#[derive(Debug, Clone)]
pub struct Session {
    plan: Plan,
    cursor: Option<StepId>,
    log: Vec<LogEntry>,
    /// When each step was first reached, for timing sections.
    entered: HashMap<StepId, SystemTime>,
    /// How many times round each repeating thing has gone already: a
    /// step or a section by its id, the plan as `None`.
    rounds: HashMap<Option<StepId>, u16>,
    /// What shuffled orders are drawn from, so the same round of the
    /// same section reads the same each time it is asked.
    seed: u64,
    /// The time of day, as time since midnight, for a repeat that ends
    /// at one. Unknown, such a repeat goes on.
    now: Option<Duration>,
}

/// A section by its id, or the plan.
type Group = Option<StepId>;

/// What going on does to the rounds counted: a new count for something
/// that comes again, `None` for something that is over.
type Rounds = Vec<(Group, Option<u16>)>;

impl Session {
    /// A session that hasn't started yet.
    pub fn new(plan: Plan) -> Self {
        Self {
            plan,
            cursor: None,
            log: Vec::new(),
            entered: HashMap::new(),
            rounds: HashMap::new(),
            seed: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(1, |since| since.as_nanos() as u64 | 1),
            now: None,
        }
    }

    /// The plan.
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// Edits the plan. If the current step is removed, the cursor moves to the
    /// step before it, so the next [`next`](Self::next) lands on what followed.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut Plan) -> R) -> R {
        let before = self.current_index();
        let result = f(&mut self.plan);
        if let (Some(index), None) = (before, self.current_index()) {
            let steps = self.plan.steps();
            self.cursor = steps[..index.min(steps.len())]
                .iter()
                .rev()
                .find(|s| !s.is_section())
                .map(|s| s.id);
        }
        result
    }

    /// Swaps in a different plan and starts over. The log is kept.
    pub fn replace_plan(&mut self, plan: Plan) {
        self.plan = plan;
        self.reset();
    }

    /// The step we're at, if the session has started.
    pub fn current(&self) -> Option<&Step> {
        self.plan.get(self.cursor?)
    }

    /// Position of the current step.
    pub fn current_index(&self) -> Option<usize> {
        self.plan.position(self.cursor?)
    }

    /// The step [`next`](Self::next) would move to, for an "up next" panel.
    pub fn peek_next(&self) -> Option<&Step> {
        let (index, _) = self.following()?;
        self.plan.steps().get(index)
    }

    /// Moves to the next step (or the first, if not started), following
    /// the plan's flow: a step, a section or the plan that repeats comes
    /// again, innermost first, and a shuffled section gives its steps in
    /// its drawn order. At the end, stays on the last step and returns
    /// `None`.
    #[allow(clippy::should_implement_trait)] // a cursor, not an iterator: it moves both ways
    pub fn next(&mut self) -> Option<&Step> {
        let (index, rounds) = self.following()?;
        for (of, round) in rounds {
            match round {
                Some(round) => self.rounds.insert(of, round),
                None => self.rounds.remove(&of),
            };
        }
        self.move_to_index(index)
    }

    /// The time of day, as time since midnight: a repeat that ends at a
    /// time needs it.
    pub fn tell_time(&mut self, since_midnight: Duration) {
        self.now = Some(since_midnight);
    }

    /// Where `next` would go, and what it does to the rounds counted: a
    /// new count, or `None` for one that is over.
    fn following(&self) -> Option<(usize, Rounds)> {
        let steps = self.plan.steps();
        let mut rounds = Vec::new();
        let Some(at) = self.current_index() else {
            return Some((self.entering(self.forward(0)?, None), rounds));
        };
        // The step itself comes again.
        let own = Some(steps[at].id);
        if self.again(steps[at].flow, self.round(own)) {
            rounds.push((own, Some(self.round(own) + 1)));
            return Some((at, rounds));
        }
        rounds.push((own, None));
        // Then the sections it is in, innermost first, and the plan.
        let groups = self
            .plan
            .within(at)
            .into_iter()
            .rev()
            .map(Some)
            .chain([None]);
        // What follows in the plan's own order, inside a section that
        // isn't over; and where the plan goes on after one that is.
        let mut inside = None;
        let mut after = at + 1;
        for group in groups {
            let flow = self.flow_of(group);
            let order = self.order(group, self.round(group));
            let place = order.iter().position(|&index| index == at);
            if place.is_none_or(|place| place + 1 < order.len()) {
                // Not over. Shuffled, its order says what follows.
                if flow.shuffle
                    && let Some(place) = place
                {
                    return Some((order[place + 1], rounds));
                }
                inside.get_or_insert(after);
                continue;
            }
            // Over: once more round, or out of it.
            if self.again(flow, self.round(group)) {
                let round = self.round(group) + 1;
                rounds.push((group, Some(round)));
                return self
                    .order(group, round)
                    .first()
                    .map(|&first| (first, rounds));
            }
            rounds.push((group, None));
            inside = None;
            after = self.span(group).1;
        }
        let index = self.forward(inside.unwrap_or(after))?;
        Some((self.entering(index, Some(at)), rounds))
    }

    /// Going to `index` from `from` in the plan's order: into a shuffled
    /// section that `from` wasn't in, its first step is the one drawn
    /// first. The outermost such section says.
    fn entering(&self, index: usize, from: Option<usize>) -> usize {
        let was_in: Vec<StepId> = from.map_or_else(Vec::new, |from| self.plan.within(from));
        let mut groups = vec![None];
        groups.extend(self.plan.within(index).into_iter().map(Some));
        groups
            .into_iter()
            .filter(|group| self.flow_of(*group).shuffle)
            .find(|group| match group {
                Some(id) => !was_in.contains(id),
                None => from.is_none(),
            })
            .and_then(|group| self.order(group, self.round(group)).first().copied())
            .unwrap_or(index)
    }

    fn round(&self, of: Group) -> u16 {
        self.rounds.get(&of).copied().unwrap_or(0)
    }

    fn flow_of(&self, group: Group) -> Flow {
        match group {
            Some(id) => self.plan.get(id).map(|step| step.flow).unwrap_or_default(),
            None => self.plan.flow,
        }
    }

    /// Where a section's steps start and end in the plan; all of it for
    /// the plan.
    fn span(&self, group: Group) -> (usize, usize) {
        match group.and_then(|id| self.plan.position(id)) {
            Some(at) => (at + 1, self.plan.group_end(at)),
            None => (0, self.plan.len()),
        }
    }

    /// The steps of a section, or the plan, in the order they come this
    /// `round`: the plan's, or a drawn one when it is shuffled.
    fn order(&self, group: Group, round: u16) -> Vec<usize> {
        let (start, end) = self.span(group);
        let steps = self.plan.steps();
        let mut order: Vec<usize> = (start..end).filter(|&i| !steps[i].is_section()).collect();
        if self.flow_of(group).shuffle {
            let of: u64 = group.map_or(0, |id| id.0 + 1);
            let mut state =
                self.seed ^ of.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ u64::from(round) << 32;
            let mut draw = move || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            };
            for last in (1..order.len()).rev() {
                order.swap(last, (draw() % (last as u64 + 1)) as usize);
            }
        }
        order
    }

    /// Whether something that has come `done` times more than once comes
    /// again.
    fn again(&self, flow: Flow, done: u16) -> bool {
        match flow.repeat {
            Repeat::Once => false,
            Repeat::Times(times) => done.saturating_add(1) < times,
            Repeat::Forever => true,
            Repeat::Until(time) => self.now.is_none_or(|now| now < time.since_midnight()),
        }
    }

    /// Moves to the previous step. At the first step, stays and returns `None`.
    pub fn prev(&mut self) -> Option<&Step> {
        let before = self.current_index()?;
        let index = self.plan.steps()[..before]
            .iter()
            .rposition(|s| !s.is_section())?;
        self.move_to_index(index)
    }

    /// Jumps to a step, or to the first step of a section.
    pub fn goto(&mut self, id: StepId) -> Option<&Step> {
        let index = self.forward(self.plan.position(id)?)?;
        self.move_to_index(index)
    }

    /// Goes back to "not started", forgetting when steps were reached.
    pub fn reset(&mut self) {
        self.cursor = None;
        self.entered.clear();
        self.rounds.clear();
    }

    /// When the first step was reached: when the service began.
    pub fn started(&self) -> Option<SystemTime> {
        self.entered.values().min().copied()
    }

    /// Which time round a repeating step or section is on, or the plan
    /// (`None`): 0 the first time.
    pub fn round_of(&self, of: Option<StepId>) -> u16 {
        self.round(of)
    }

    /// When a step was first reached in this session.
    pub fn entered(&self, id: StepId) -> Option<SystemTime> {
        self.entered.get(&id).copied()
    }

    /// Records that `resource` went on stage. Off-plan resources shown during
    /// a [`Action::Live`] step are also remembered in that step.
    pub fn record(&mut self, resource: Resource, planned: bool) {
        if !planned {
            let current = self.cursor.and_then(|id| self.plan.get_mut(id));
            if let Some(Step {
                action: Action::Live { shown, .. },
                ..
            }) = current
                && !shown.iter().any(|r| r.id == resource.id)
            {
                shown.push(resource.clone());
            }
        }
        self.log.push(LogEntry {
            at: SystemTime::now(),
            step: self.cursor,
            resource,
            planned,
        });
    }

    /// Inserts a step right after `after`. Without one (or if it's gone),
    /// right after the current step, or at the end before the session
    /// starts, so a plan built ahead of time keeps its order. Handy for
    /// turning something found live into part of the plan.
    pub fn insert_after(
        &mut self,
        after: Option<StepId>,
        label: impl Into<String>,
        action: Action,
    ) -> StepId {
        let index = after
            .and_then(|id| self.plan.position(id))
            .or(self.current_index())
            .map_or(self.plan.len(), |i| i + 1);
        self.plan.insert(index, label, action)
    }

    /// Everything shown so far, oldest first. Useful for song-usage reports.
    pub fn log(&self) -> &[LogEntry] {
        &self.log
    }

    /// The first step at or after `index` that isn't a section marker.
    fn forward(&self, index: usize) -> Option<usize> {
        let steps = self.plan.steps();
        (index..steps.len()).find(|&i| !steps[i].is_section())
    }

    fn move_to_index(&mut self, index: usize) -> Option<&Step> {
        let step = self.plan.steps().get(index)?;
        self.cursor = Some(step.id);
        self.entered.entry(step.id).or_insert_with(SystemTime::now);
        Some(step)
    }
}

#[cfg(test)]
mod tests {
    use lantern_resource::{Kind, Location, ResourceId, SourceId};

    use super::*;

    fn resource(title: &str) -> Resource {
        let source = SourceId::new("test").unwrap();
        Resource::new(
            ResourceId::new(&source, title),
            title,
            Kind::Image,
            Location::Url(title.into()),
        )
    }

    fn label(step: Option<&Step>) -> Option<&str> {
        step.map(|s| s.label.as_str())
    }

    fn session() -> Session {
        let mut plan = Plan::new("Sunday");
        plan.show(resource("Welcome"));
        plan.live("Worship", None);
        plan.push("Prayer", Action::Blank);
        Session::new(plan)
    }

    #[test]
    fn walks_forward_and_back() {
        let mut s = session();
        assert_eq!(label(s.current()), None);
        assert_eq!(label(s.peek_next()), Some("Welcome"));
        assert_eq!(label(s.next()), Some("Welcome"));
        assert_eq!(label(s.next()), Some("Worship"));
        assert_eq!(label(s.next()), Some("Prayer"));
        assert_eq!(label(s.next()), None);
        assert_eq!(label(s.current()), Some("Prayer"));
        assert_eq!(label(s.prev()), Some("Worship"));
        assert_eq!(label(s.prev()), Some("Welcome"));
        assert_eq!(label(s.prev()), None);
        assert_eq!(label(s.current()), Some("Welcome"));
    }

    #[test]
    fn cursor_survives_edits() {
        let mut s = session();
        s.next();
        s.next(); // Worship
        s.edit(|plan| plan.insert(0, "Intro", Action::Note));
        assert_eq!(label(s.current()), Some("Worship"));

        let worship = s.current().unwrap().id;
        s.edit(|plan| plan.remove(worship));
        assert_eq!(label(s.current()), Some("Welcome"));
        assert_eq!(label(s.next()), Some("Prayer"));
    }

    #[test]
    fn removing_the_first_step_while_on_it_restarts() {
        let mut s = session();
        let first = s.next().unwrap().id;
        s.edit(|plan| plan.remove(first));
        assert_eq!(label(s.current()), None);
        assert_eq!(label(s.next()), Some("Worship"));
    }

    #[test]
    fn live_slots_remember_what_was_shown() {
        let mut s = session();
        s.next();
        s.record(resource("Welcome"), true);
        s.next(); // Worship
        s.record(resource("Song A"), false);
        s.record(resource("Song A"), false);
        s.record(resource("Song B"), false);

        let Action::Live { shown, .. } = &s.current().unwrap().action else {
            panic!()
        };
        let titles: Vec<_> = shown.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["Song A", "Song B"]);
        assert_eq!(s.log().len(), 4);
        assert!(s.log()[0].planned && !s.log()[1].planned);
    }

    #[test]
    fn insert_after_current() {
        let mut s = session();
        s.next(); // Welcome
        s.insert_after(
            None,
            "Announcement",
            Action::Show {
                resource: resource("Ad"),
            },
        );
        assert_eq!(label(s.next()), Some("Announcement"));
    }

    #[test]
    fn stepping_passes_over_sections() {
        let mut plan = Plan::new("Sunday");
        plan.section("Alabanza", None);
        plan.show(resource("Song"));
        let word = plan.section("Palabra", None);
        plan.push("Sermon", Action::Blank);
        let mut s = Session::new(plan);
        assert_eq!(label(s.peek_next()), Some("Song"));
        assert_eq!(label(s.next()), Some("Song"));
        assert_eq!(label(s.next()), Some("Sermon"));
        assert_eq!(label(s.prev()), Some("Song"));
        assert_eq!(
            label(s.goto(word)),
            Some("Sermon"),
            "a section jumps to its first step"
        );
        assert!(s.entered(s.current().unwrap().id).is_some());

        let sermon = s.current().unwrap().id;
        s.edit(|plan| plan.remove(sermon));
        assert_eq!(label(s.current()), Some("Song"), "back past the marker");
        s.reset();
        assert!(s.entered(sermon).is_none());
    }

    #[test]
    fn before_the_start_new_steps_go_at_the_end() {
        let mut s = session();
        s.insert_after(None, "Benediction", Action::Note);
        let last = s.plan().steps().last().unwrap();
        assert_eq!(last.label, "Benediction");
        let welcome = s.plan().steps()[0].id;
        s.insert_after(Some(welcome), "Intro", Action::Note);
        assert_eq!(
            s.plan().steps()[1].label,
            "Intro",
            "right after the one asked for"
        );
    }

    /// Music { A, B }, Sermon { C }.
    fn two_sections() -> (Session, StepId, StepId) {
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        plan.show(resource("A"));
        plan.show(resource("B"));
        let sermon = plan.section("Sermon", None);
        plan.show(resource("C"));
        (Session::new(plan), music, sermon)
    }

    fn walk(session: &mut Session, steps: usize) -> String {
        (0..steps)
            .map(|_| label(session.next()).unwrap_or("-").to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_section_that_repeats_comes_again_then_the_plan_goes_on() {
        let (mut session, music, _) = two_sections();
        session.edit(|plan| plan.get_mut(music).unwrap().flow.repeat = Repeat::Times(2));
        assert_eq!(walk(&mut session, 6), "A B A B C -");
    }

    #[test]
    fn a_loop_round_some_steps_comes_again_and_what_follows_is_out_of_it() {
        // Music: A B D, Sermon: C. A loop round A and B, twice.
        let (mut session, music, _) = two_sections();
        session.edit(|plan| {
            let (a, b) = (plan.steps()[1].id, plan.steps()[2].id);
            let d = plan.insert(3, "D", Action::Blank);
            let looped = plan.loop_over(b, a).expect("a loop round A and B");
            assert!(plan.get(looped).unwrap().is_loop());
            assert_eq!(
                plan.within(plan.position(d).unwrap()),
                [music],
                "D is out of it"
            );
        });
        assert_eq!(walk(&mut session, 7), "A B A B D C -");
        // Music twice too: the loop is gone round again each time.
        let (mut session, music, _) = two_sections();
        session.edit(|plan| {
            let (a, b) = (plan.steps()[1].id, plan.steps()[2].id);
            plan.insert(3, "D", Action::Blank);
            plan.loop_over(a, b);
            plan.get_mut(music).unwrap().flow.repeat = Repeat::Times(2);
        });
        assert_eq!(walk(&mut session, 12), "A B A B D A B A B D C -");
    }

    #[test]
    fn a_step_that_repeats_comes_again_before_its_section_does() {
        let (mut session, music, _) = two_sections();
        session.edit(|plan| {
            let b = plan.steps()[2].id;
            plan.get_mut(b).unwrap().flow.repeat = Repeat::Times(2);
            plan.get_mut(music).unwrap().flow.repeat = Repeat::Times(2);
        });
        // B twice each time round, and Music twice round.
        assert_eq!(walk(&mut session, 7), "A B B A B B C");
    }

    #[test]
    fn a_plan_that_repeats_starts_over_and_forever_never_ends() {
        let (mut session, _, sermon) = two_sections();
        session.edit(|plan| plan.flow.repeat = Repeat::Times(2));
        assert_eq!(walk(&mut session, 7), "A B C A B C -");
        let (mut session, ..) = two_sections();
        session.edit(|plan| plan.get_mut(sermon).unwrap().flow.repeat = Repeat::Forever);
        assert_eq!(walk(&mut session, 6), "A B C C C C");
        // Going somewhere else is the way out.
        let first = session.plan().steps()[1].id;
        assert_eq!(label(session.goto(first)), Some("A"));
    }

    #[test]
    fn a_repeat_until_a_time_ends_when_the_time_has_come() {
        use crate::timing::TimeOfDay;
        let (mut session, music, _) = two_sections();
        let at = |text: &str| TimeOfDay::parse(text).unwrap();
        session.edit(|plan| plan.get_mut(music).unwrap().flow.repeat = Repeat::Until(at("10:30")));
        session.tell_time(at("10:00").since_midnight());
        assert_eq!(walk(&mut session, 4), "A B A B");
        // What is on is finished, then the plan goes on.
        session.tell_time(at("10:31").since_midnight());
        assert_eq!(walk(&mut session, 2), "C -");
    }

    #[test]
    fn a_shuffled_section_gives_each_step_once_a_round_and_up_next_agrees() {
        let mut plan = Plan::new("Before");
        let music = plan.section("Music", None);
        for title in ["A", "B", "C", "D", "E"] {
            plan.show(resource(title));
        }
        plan.section("Sermon", None);
        plan.show(resource("Z"));
        plan.get_mut(music).unwrap().flow = Flow {
            repeat: Repeat::Times(2),
            shuffle: true,
        };
        let mut session = Session::new(plan);
        let mut rounds = Vec::new();
        for _ in 0..2 {
            let mut round = Vec::new();
            for _ in 0..5 {
                let coming = label(session.peek_next()).map(str::to_string);
                let came = label(session.next()).map(str::to_string);
                assert_eq!(coming, came, "up next is what comes");
                round.push(came.unwrap());
            }
            let mut sorted = round.clone();
            sorted.sort();
            assert_eq!(sorted, ["A", "B", "C", "D", "E"], "each once");
            rounds.push(round);
        }
        // Out of the section when its rounds are over.
        assert_eq!(label(session.next()), Some("Z"));
    }

    #[test]
    fn going_on_carries_down_and_a_step_can_say_otherwise() {
        let (mut session, music, _) = two_sections();
        session.edit(|plan| {
            plan.get_mut(music).unwrap().advance = Some(true);
            let b = plan.steps()[2].id;
            plan.get_mut(b).unwrap().advance = Some(false);
        });
        let plan = session.plan();
        assert!(plan.goes_on(1), "A does as Music says");
        assert!(!plan.goes_on(2), "B says it waits");
        assert!(!plan.goes_on(4), "C is in Sermon, which says nothing");
        let mut plan = plan.clone();
        plan.advance = true;
        assert!(plan.goes_on(4), "then the plan says");
    }
}
