use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use lantern_resource::Resource;
use serde::{Deserialize, Serialize};

use crate::{Action, Ending, Flow, FlowOf, Place, Plan, Repeat, SpanId, Step, StepId};

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
    /// step by itself, a span, or the plan.
    rounds: HashMap<FlowOf, u16>,
    /// What shuffled orders are drawn from, so the same round of the
    /// same span reads the same each time it is asked.
    seed: u64,
    /// The time of day, as time since midnight, for a repeat that ends
    /// at one. Unknown, such a repeat goes on.
    now: Option<Duration>,
}

/// A step still to come, as the service will reach it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coming {
    /// The step.
    pub step: StepId,
    /// Which time round it comes in, from 0: of the step itself when it
    /// repeats, else of the innermost loop it is in, else of the plan.
    pub round: u16,
    /// What that round is of: the step itself, a loop, or the plan;
    /// `None` where nothing it is in comes again.
    pub of: Option<FlowOf>,
    /// How long from now until it starts.
    pub after: Duration,
    /// Whether what comes before it goes on to it by itself; if not, it
    /// waits for the operator.
    pub joined: bool,
}

/// What the steps go round in: a span by its id, or the plan.
type Group = Option<SpanId>;

/// What going on does to the rounds counted: a new count for something
/// that comes again, `None` for something that is over.
type Rounds = Vec<(FlowOf, Option<u16>)>;

/// A span, or the plan, as what its rounds are counted for.
fn counted(group: Group) -> FlowOf {
    group.map_or(FlowOf::Plan, FlowOf::Span)
}

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
        // Each section keeps the colour it is shown in, whatever the edit
        // moves; an edit that changes nothing changes no colour either.
        let was = self.plan.clone();
        let kept = self.plan.keep_section_colors();
        let result = f(&mut self.plan);
        if !kept.is_empty() {
            let mut bare = self.plan.clone();
            for id in kept {
                if let Some(Action::Section { color, .. }) = bare.get_mut(id).map(|s| &mut s.action)
                {
                    *color = None;
                }
            }
            if bare == was {
                self.plan = was;
            }
        }
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
    /// the plan's flow: a step, a span or the plan that repeats comes
    /// again, innermost first, and a shuffled span gives its steps in
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

    /// The road ahead: what comes after the step that is on, in the
    /// order the service will reach it, at most `most` of them. `length`
    /// is how long a step takes, and `left` how long the step that is on
    /// has still to go. The time of day moves on with the steps, so a
    /// repeat until a time ends where it will.
    pub fn ahead(
        &self,
        length: impl Fn(&Step) -> Duration,
        left: Duration,
        most: usize,
    ) -> Vec<Coming> {
        let mut road = Session {
            plan: self.plan.clone(),
            cursor: self.cursor,
            log: Vec::new(),
            entered: HashMap::new(),
            rounds: self.rounds.clone(),
            seed: self.seed,
            now: self.now,
        };
        let base = self.now;
        let (mut start, mut after) = match self.cursor {
            Some(_) => (Duration::ZERO, left),
            None => (Duration::ZERO, Duration::ZERO),
        };
        let mut found = Vec::new();
        while found.len() < most {
            // Cut off there and then: what is on ends at that time.
            if let (Some(base), Some(cut)) = (base, road.cut_at()) {
                after = after.min(cut.saturating_sub(base).max(start));
            }
            road.now = base.map(|base| base + after);
            let joined = road.current_index().is_some_and(|at| road.plan.goes_on(at));
            let Some(step) = road.next() else {
                break;
            };
            let id = step.id;
            let took = length(step);
            found.push(Coming {
                step: id,
                round: road.counting(id).map_or(0, |(_, round)| round),
                of: road.counting(id).map(|(of, _)| of),
                after,
                joined,
            });
            start = after;
            after += took;
        }
        found
    }

    /// The time of day, as time since midnight, the step that is on is
    /// to be cut off at: the soonest of what it is in that ends there and
    /// then at a time, as early as it takes to fade out.
    fn cut_at(&self) -> Option<Duration> {
        let at = self.current_index()?;
        let step = &self.plan.steps()[at];
        let fade = step
            .fade
            .fade_out
            .map_or(Duration::ZERO, |fade| fade.duration());
        let groups = self.plan.spans_at(at).into_iter().map(Some).chain([None]);
        std::iter::once(step.flow)
            .chain(groups.map(|group| self.flow_of(group)))
            .filter_map(|flow| match (flow.repeat, flow.ends) {
                (Repeat::Until(time), Ending::Sharp) => {
                    Some(time.since_midnight().saturating_sub(fade))
                }
                _ => None,
            })
            .min()
    }

    /// Which time round the step `id` is on, and of what, as
    /// [`Coming::round`] and [`Coming::of`] say.
    fn counting(&self, id: StepId) -> Option<(FlowOf, u16)> {
        let at = self.plan.position(id)?;
        let repeats = |of: FlowOf| {
            self.plan
                .flow_of(of)
                .is_some_and(|flow| flow.repeat != Repeat::Once)
        };
        let own = FlowOf::Step(id);
        if self.plan.steps()[at].flow.repeat != Repeat::Once {
            return Some((own, self.round(own)));
        }
        self.plan
            .spans_at(at)
            .into_iter()
            .rev()
            .map(FlowOf::Span)
            .chain([FlowOf::Plan])
            .find(|of| repeats(*of))
            .map(|of| (of, self.round(of)))
    }

    /// The time of day, as time since midnight: a repeat that ends at a
    /// time needs it.
    pub fn tell_time(&mut self, since_midnight: Duration) {
        self.now = Some(since_midnight);
    }

    /// The time of day last told, as time since midnight.
    pub fn time_of_day(&self) -> Option<Duration> {
        self.now
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
        let own = FlowOf::Step(steps[at].id);
        if self.again(steps[at].flow, self.round(own)) {
            rounds.push((own, Some(self.round(own) + 1)));
            return Some((at, rounds));
        }
        rounds.push((own, None));
        // Then the loops and drawn orders it is in, innermost first, and
        // the plan. A join changes nothing of where it goes.
        let groups = self
            .plan
            .spans_at(at)
            .into_iter()
            .rev()
            .filter(|id| self.plan.span(*id).is_some_and(|span| span.flow.shapes()))
            .map(Some)
            .chain([None]);
        // What follows in the plan's own order, inside a span that
        // isn't over; and where the plan goes on after one that is.
        let mut inside = None;
        let mut after = at + 1;
        for group in groups {
            let flow = self.flow_of(group);
            let of = counted(group);
            let order = self.order(group, self.round(of));
            let place = order.iter().position(|&index| index == at);
            // Its time has come and it doesn't finish the round: it is
            // over where it is.
            let cut = self.past_its_time(flow, Ending::Step);
            // More of it is to come: in a drawn order, after this step's
            // place in it; in the plan's, after what is over already.
            let more = match flow.shuffle {
                true => place.is_none_or(|place| place + 1 < order.len()),
                false => order.iter().any(|&index| index >= after),
            };
            if !cut && more {
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
            if self.again(flow, self.round(of)) {
                let round = self.round(of) + 1;
                rounds.push((of, Some(round)));
                return self
                    .order(group, round)
                    .first()
                    .map(|&first| (first, rounds));
            }
            rounds.push((of, None));
            inside = None;
            after = self.reach(group).1;
        }
        let index = self.forward(inside.unwrap_or(after))?;
        Some((self.entering(index, Some(at)), rounds))
    }

    /// Going to `index` from `from` in the plan's order: into a shuffled
    /// span that `from` wasn't in, its first step is the one drawn
    /// first. The outermost such span says.
    fn entering(&self, index: usize, from: Option<usize>) -> usize {
        let was_in: Vec<SpanId> = from.map_or_else(Vec::new, |from| self.plan.spans_at(from));
        let mut groups = vec![None];
        groups.extend(self.plan.spans_at(index).into_iter().map(Some));
        groups
            .into_iter()
            .filter(|group| self.flow_of(*group).shuffle)
            .find(|group| match group {
                Some(id) => !was_in.contains(id),
                None => from.is_none(),
            })
            .and_then(|group| {
                self.order(group, self.round(counted(group)))
                    .first()
                    .copied()
            })
            .unwrap_or(index)
    }

    /// How many times round `of` has gone already. A section's heading
    /// counts as the span attached to the section.
    fn round(&self, of: FlowOf) -> u16 {
        let of = match of {
            FlowOf::Step(id) => self
                .plan
                .section_span(id)
                .map_or(of, |span| FlowOf::Span(span.id)),
            _ => of,
        };
        self.rounds.get(&of).copied().unwrap_or(0)
    }

    fn flow_of(&self, group: Group) -> Flow {
        self.plan.flow_of(counted(group)).unwrap_or_default()
    }

    /// Where a span's rows start and end in the plan; all of it for the
    /// plan.
    fn reach(&self, group: Group) -> (usize, usize) {
        match group {
            Some(id) => self
                .plan
                .span_range(id)
                .map_or((0, 0), |(first, last)| (first, last + 1)),
            None => (0, self.plan.len()),
        }
    }

    /// The steps of a span, or the plan, in the order they come this
    /// `round`: the plan's, or a drawn one when it is shuffled.
    fn order(&self, group: Group, round: u16) -> Vec<usize> {
        let (start, end) = self.reach(group);
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

    /// Whether `flow` comes again until a time of day that has come,
    /// and ends no later than `ending` says: with the step that is on
    /// ([`Ending::Step`], or sooner), or there and then.
    fn past_its_time(&self, flow: Flow, ending: Ending) -> bool {
        let ends = !matches!(
            (flow.ends, ending),
            (Ending::Round, _) | (Ending::Step, Ending::Sharp)
        );
        match flow.repeat {
            Repeat::Until(time) if ends => self.has_come(flow, time),
            _ => false,
        }
    }

    /// Whether `time` has come for `flow`. One that cuts there and then
    /// counts it come as early as the step that is on takes to fade out,
    /// so that is silent by then.
    fn has_come(&self, flow: Flow, time: crate::timing::TimeOfDay) -> bool {
        let early = match flow.ends {
            Ending::Sharp => self
                .current()
                .and_then(|step| step.fade.fade_out)
                .map_or(Duration::ZERO, |fade| fade.duration()),
            Ending::Round | Ending::Step => Duration::ZERO,
        };
        self.now
            .is_some_and(|now| now + early >= time.since_midnight())
    }

    /// Whether what is on should be cut off now: it, or something it is
    /// in, comes again until a time of day, ends there and then, and the
    /// time has come. [`next`](Self::next) then leaves it.
    pub fn cut_now(&self) -> bool {
        let Some(at) = self.current_index() else {
            return false;
        };
        let own = self.plan.steps()[at].flow;
        let groups = self.plan.spans_at(at).into_iter().map(Some).chain([None]);
        self.past_its_time(own, Ending::Sharp)
            || groups
                .into_iter()
                .any(|group| self.past_its_time(self.flow_of(group), Ending::Sharp))
    }

    /// Whether something that has come `done` times more than once comes
    /// again.
    fn again(&self, flow: Flow, done: u16) -> bool {
        match flow.repeat {
            Repeat::Once => false,
            Repeat::Times(times) => done.saturating_add(1) < times,
            Repeat::Forever => true,
            Repeat::Until(time) => self.now.is_none() || !self.has_come(flow, time),
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

    /// Which time round a repeating step, span or plan is on: 0 the
    /// first time. A section's heading says it for the span attached to
    /// the section.
    pub fn round_of(&self, of: impl Into<FlowOf>) -> u16 {
        self.round(of.into())
    }

    /// The steps that have come already this round of each shuffled span
    /// the step that is on is in: some stand after it in the plan.
    pub fn drawn_before(&self) -> Vec<StepId> {
        let Some(at) = self.cursor.and_then(|id| self.plan.position(id)) else {
            return Vec::new();
        };
        let mut groups = vec![None];
        groups.extend(self.plan.spans_at(at).into_iter().map(Some));
        let mut before = Vec::new();
        for group in groups
            .into_iter()
            .filter(|group| self.flow_of(*group).shuffle)
        {
            let order = self.order(group, self.round(counted(group)));
            if let Some(k) = order.iter().position(|&index| index == at) {
                before.extend(order[..k].iter().map(|&index| self.plan.steps()[index].id));
            }
        }
        before
    }

    /// What going on from the step that is on goes back round: the step
    /// itself coming again, the span whose round it ends, or the plan.
    /// `None` when going on goes forward, or before the start. It is what
    /// [`next`](Self::next) would do now, so it changes as the time of
    /// day or the plan does.
    pub fn follows_back(&self) -> Option<FlowOf> {
        self.current_index()?;
        let (_, rounds) = self.following()?;
        rounds.into_iter().find_map(|(of, round)| round.map(|_| of))
    }

    /// Which time round `of` is on, counted from 0, and of how many in
    /// all: `None` for a repeat with no number to it (until moved on, or
    /// until a time). A section's heading says it for the span attached
    /// to the section.
    pub fn rounds(&self, of: impl Into<FlowOf>) -> (u16, Option<u16>) {
        let of = of.into();
        let repeat = self.plan.flow_of(of).unwrap_or_default().repeat;
        let of_all = match repeat {
            Repeat::Once => Some(1),
            Repeat::Times(times) => Some(times.max(1)),
            Repeat::Forever | Repeat::Until(_) => None,
        };
        (self.round(of), of_all)
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
    /// turning something found live into part of the plan. It is in the
    /// section the step it follows is in, though that section ended at
    /// that step; right after a heading, in that section.
    pub fn insert_after(
        &mut self,
        after: Option<StepId>,
        label: impl Into<String>,
        action: Action,
    ) -> StepId {
        let Some(at) = after
            .and_then(|id| self.plan.position(id))
            .or(self.current_index())
        else {
            return self.plan.insert(self.plan.len(), label, action);
        };
        // In the same section as the step it follows, though that
        // section ended at it; right after a heading, in that section.
        let step = &self.plan.steps()[at];
        let section = match step.is_section() {
            true => Some(step.id),
            false => self.plan.within(at).last().copied(),
        };
        let to = Place {
            index: at + 1,
            section,
        };
        let label = label.into();
        match self.plan.insert_at(to, label.clone(), action.clone()) {
            Ok(id) => id,
            Err(_) => self.plan.insert(at + 1, label, action),
        }
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
    fn an_edit_keeps_section_colours_unless_it_changes_nothing() {
        let mut plan = Plan::new("Sunday");
        plan.section("Music", None);
        plan.show(resource("Song"));
        let word = plan.section("Word", None);
        let mut s = Session::new(plan.clone());
        s.edit(|_| ());
        assert_eq!(s.plan(), &plan);
        s.edit(|plan| plan.show(resource("Reading")));
        assert_eq!(s.plan().get(word).and_then(Step::color), Some(1));
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
        session.edit(|plan| plan.loop_section(music).unwrap());
        assert_eq!(walk(&mut session, 3), "A B A");
        assert_eq!(session.round_of(music), 1, "the second time round");
        assert_eq!(walk(&mut session, 3), "B C -");
    }

    #[test]
    fn a_loop_round_some_steps_comes_again_and_what_follows_is_out_of_it() {
        // Music: A B D, Sermon: C. A loop round A and B, twice.
        let (mut session, ..) = two_sections();
        session.edit(|plan| {
            let (a, b) = (plan.steps()[1].id, plan.steps()[2].id);
            let d = plan.insert(3, "D", Action::Blank);
            let looped = plan.loop_over(b, a).expect("a loop round A and B");
            assert_eq!(plan.spans_at(plan.position(a).unwrap()), [looped]);
            assert!(
                plan.spans_at(plan.position(d).unwrap()).is_empty(),
                "D is out of it"
            );
        });
        assert_eq!(walk(&mut session, 7), "A B A B D C -");
        // Music twice too: the loop is gone round again each time.
        let (mut session, music, _) = two_sections();
        session.edit(|plan| {
            let (a, b) = (plan.steps()[1].id, plan.steps()[2].id);
            plan.insert(3, "D", Action::Blank);
            plan.loop_over(a, b).unwrap();
            plan.loop_section(music).unwrap();
        });
        assert_eq!(walk(&mut session, 12), "A B A B D A B A B D C -");
    }

    #[test]
    fn until_a_time_ends_with_the_round_with_the_step_or_there_and_then() {
        use crate::timing::TimeOfDay;
        let at = |text: &str| TimeOfDay::parse(text).unwrap().since_midnight();
        let until = |ends: Ending| {
            let (mut session, music, _) = two_sections();
            session.edit(|plan| {
                let flow = Flow {
                    repeat: Repeat::Until(TimeOfDay::parse("10:30").unwrap()),
                    ends,
                    ..Flow::default()
                };
                plan.set_flow(music, flow).unwrap();
            });
            session.tell_time(at("10:00"));
            session
        };
        // Before the time, each goes round again.
        for ends in [Ending::Round, Ending::Step, Ending::Sharp] {
            let mut session = until(ends);
            assert_eq!(walk(&mut session, 3), "A B A", "{ends:?}");
            assert!(!session.cut_now());
        }
        // The time comes while A is on. The round is finished: B, then on.
        let mut session = until(Ending::Round);
        walk(&mut session, 1);
        session.tell_time(at("10:31"));
        assert!(!session.cut_now());
        assert_eq!(walk(&mut session, 2), "B C");
        // With the step: A is let end, and B is left out.
        let mut session = until(Ending::Step);
        walk(&mut session, 1);
        session.tell_time(at("10:31"));
        assert!(!session.cut_now(), "A is not cut off");
        assert_eq!(walk(&mut session, 1), "C");
        // There and then: A is to be cut, and going on leaves Music.
        let mut session = until(Ending::Sharp);
        walk(&mut session, 1);
        assert!(!session.cut_now());
        session.tell_time(at("10:30"));
        assert!(session.cut_now());
        assert_eq!(walk(&mut session, 1), "C");
        assert!(!session.cut_now(), "out of it, nothing more to cut");
    }

    fn road(session: &Session, left: u64, most: usize) -> String {
        session
            .ahead(|_| Duration::from_secs(60), Duration::from_secs(left), most)
            .iter()
            .map(|coming| {
                let label = &session.plan().get(coming.step).unwrap().label;
                format!("{label}{}@{}", coming.round, coming.after.as_secs())
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn the_road_ahead_is_what_going_on_reaches_and_when() {
        let (mut session, music, _) = two_sections();
        session.edit(|plan| plan.loop_section(music).unwrap());
        assert_eq!(road(&session, 0, 10), "A0@0 B0@60 A1@120 B1@180 C0@240");
        session.next();
        assert_eq!(road(&session, 25, 10), "B0@25 A1@85 B1@145 C0@205");
        // It does not move the session.
        assert_eq!(label(session.current()), Some("A"));
    }

    #[test]
    fn the_road_ahead_of_a_drawn_order_is_the_order_it_will_come_in() {
        let mut plan = Plan::new("Before");
        let music = plan.section("Music", None);
        for title in ["A", "B", "C", "D", "E"] {
            plan.show(resource(title));
        }
        plan.section("Sermon", None);
        plan.show(resource("Z"));
        let mut session = Session::new(plan);
        let twice_drawn = Flow {
            repeat: Repeat::Times(2),
            shuffle: true,
            ..Flow::default()
        };
        session.edit(|plan| plan.set_flow(music, twice_drawn).unwrap());
        session.next();
        session.next();
        let ahead: Vec<StepId> = session
            .ahead(|_| Duration::from_secs(60), Duration::ZERO, 100)
            .iter()
            .map(|coming| coming.step)
            .collect();
        let mut going = session.clone();
        let went: Vec<StepId> = std::iter::from_fn(|| going.next().map(|step| step.id)).collect();
        assert_eq!(ahead, went);
        assert_eq!(ahead.len(), 9, "the rest of this round, the next, then Z");
    }

    #[test]
    fn the_road_ahead_ends_a_repeat_until_a_time_where_the_time_comes() {
        use crate::timing::TimeOfDay;
        let at = |text: &str| TimeOfDay::parse(text).unwrap();
        let until = |ends: Ending| {
            let (mut session, music, _) = two_sections();
            let flow = Flow {
                repeat: Repeat::Until(at("10:30")),
                ends,
                ..Flow::default()
            };
            session.edit(|plan| plan.set_flow(music, flow).unwrap());
            // Five minutes to go, and each step a minute.
            session.tell_time(at("10:25").since_midnight());
            session.next();
            session
        };
        // A on now, ends at 10:26: B, A, B, A to 10:30, then B finishes
        // the round, and C.
        assert_eq!(
            road(&until(Ending::Round), 60, 10),
            "B0@60 A1@120 B1@180 A2@240 B2@300 C0@360"
        );
        // With the step: B ends at 10:30, C follows it.
        assert_eq!(
            road(&until(Ending::Step), 60, 10),
            "B0@60 A1@120 B1@180 A2@240 C0@300"
        );
        // There and then: the same here, as a step ends at 10:30 anyway.
        assert_eq!(
            road(&until(Ending::Sharp), 60, 10),
            "B0@60 A1@120 B1@180 A2@240 C0@300"
        );
        // Cut there and then mid-step: C starts at 10:30 sharp.
        let mut session = until(Ending::Sharp);
        session.tell_time(at("10:24").since_midnight() + Duration::from_secs(30));
        assert_eq!(
            road(&session, 60, 10),
            "B0@60 A1@120 B1@180 A2@240 B2@300 C0@330"
        );
    }

    #[test]
    fn the_road_ahead_says_where_it_waits_and_stops_at_most() {
        let (mut session, ..) = two_sections();
        session.edit(|plan| plan.flow.repeat = Repeat::Forever);
        assert_eq!(road(&session, 0, 4), "A0@0 B0@60 C0@120 A1@180");
        let mut joined = session.clone();
        joined.edit(|plan| plan.flow.joined = true);
        joined.next();
        let ahead = joined.ahead(|_| Duration::from_secs(60), Duration::ZERO, 2);
        assert!(ahead.iter().all(|coming| coming.joined));
        session.next();
        let ahead = session.ahead(|_| Duration::from_secs(60), Duration::ZERO, 2);
        assert!(ahead.iter().all(|coming| !coming.joined));
    }

    #[test]
    fn a_step_that_repeats_comes_again_before_its_section_does() {
        let (mut session, music, _) = two_sections();
        session.edit(|plan| {
            let b = plan.steps()[2].id;
            plan.get_mut(b).unwrap().flow.repeat = Repeat::Times(2);
            plan.loop_section(music).unwrap();
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
        let forever = Flow {
            repeat: Repeat::Forever,
            ..Flow::default()
        };
        session.edit(|plan| plan.set_flow(sermon, forever).unwrap());
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
        let until = Flow {
            repeat: Repeat::Until(at("10:30")),
            ..Flow::default()
        };
        session.edit(|plan| plan.set_flow(music, until).unwrap());
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
        let twice_drawn = Flow {
            repeat: Repeat::Times(2),
            shuffle: true,
            ..Flow::default()
        };
        plan.set_flow(music, twice_drawn).unwrap();
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
    fn what_a_shuffle_has_given_this_round_is_known_wherever_it_stands() {
        let mut plan = Plan::new("Before");
        let music = plan.section("Music", None);
        for title in ["A", "B", "C", "D", "E"] {
            plan.show(resource(title));
        }
        plan.shuffle_section(music).unwrap();
        let mut session = Session::new(plan);
        let mut came = Vec::new();
        for _ in 0..4 {
            came.push(session.next().unwrap().id);
            let before = session.drawn_before();
            assert_eq!(before, came[..came.len() - 1], "what came, in order");
        }
    }

    /// Music { A, B }, Sermon { C, D }, Close { E }.
    fn three_sections() -> (Session, [StepId; 5]) {
        let mut plan = Plan::new("Sabbath");
        plan.section("Music", None);
        let a = plan.show(resource("A"));
        let b = plan.show(resource("B"));
        plan.section("Sermon", None);
        let c = plan.show(resource("C"));
        let d = plan.show(resource("D"));
        plan.section("Close", None);
        let e = plan.show(resource("E"));
        (Session::new(plan), [a, b, c, d, e])
    }

    #[test]
    fn a_loop_across_a_heading_goes_round_from_one_section_into_the_next() {
        // From B in Music to C in Sermon, twice.
        let (mut session, [_, b, c, ..]) = three_sections();
        let looped = session.edit(|plan| plan.loop_over(b, c).unwrap());
        assert_eq!(walk(&mut session, 2), "A B");
        assert_eq!(session.follows_back(), None, "B goes on to C");
        assert_eq!(walk(&mut session, 1), "C");
        assert_eq!(
            label(session.peek_next()),
            Some("B"),
            "back over the heading"
        );
        assert_eq!(session.follows_back(), Some(FlowOf::Span(looped)));
        assert_eq!(session.rounds(looped), (0, Some(2)));
        assert_eq!(walk(&mut session, 2), "B C");
        assert_eq!(session.round_of(looped), 1);
        assert_eq!(session.rounds(looped), (1, Some(2)));
        assert_eq!(session.follows_back(), None, "the last time round");
        assert_eq!(walk(&mut session, 3), "D E -");
        // Round whole sections too: from A to D, with Sermon's heading
        // among them, three times.
        let (mut session, [a, _, _, d, _]) = three_sections();
        session.edit(|plan| {
            let looped = plan.loop_over(a, d).unwrap();
            let thrice = Flow {
                repeat: Repeat::Times(3),
                ..Flow::default()
            };
            plan.set_flow(looped, thrice).unwrap();
        });
        assert_eq!(walk(&mut session, 14), "A B C D A B C D A B C D E -");
    }

    #[test]
    fn a_span_in_a_span_goes_round_each_time_the_outer_one_does() {
        // B and C twice, inside A to D twice, the inner one across the
        // heading; then Sermon's own span round C and D is refused, for
        // it would cut the inner one in two.
        let (mut session, [a, b, c, d, _]) = three_sections();
        session.edit(|plan| {
            let inner = plan.loop_over(b, c).unwrap();
            plan.loop_over(a, d).unwrap();
            let sermon = plan.steps()[3].id;
            assert_eq!(
                plan.loop_section(sermon),
                Err(crate::SpanError::HalfOver(inner))
            );
        });
        assert_eq!(
            walk(&mut session, 14),
            "A B C B C D A B C B C D E -",
            "the inner loop starts over with each outer round"
        );
        // A drawn order in a loop: each round of the loop gives each of
        // its steps once, wherever in the plan the last one drawn sits.
        let mut plan = Plan::new("Before");
        let ids = ["A", "B", "C", "D", "E"].map(|name| plan.show(resource(name)));
        plan.shuffle_over(ids[1], ids[3]).unwrap();
        plan.loop_over(ids[0], ids[3]).unwrap();
        let mut session = Session::new(plan);
        for round in 0..2 {
            assert_eq!(label(session.next()), Some("A"), "round {round}");
            let mut drawn: Vec<String> = (0..3)
                .map(|_| {
                    let coming = label(session.peek_next()).map(str::to_string);
                    let came = label(session.next()).map(str::to_string);
                    assert_eq!(coming, came, "up next is what comes");
                    came.unwrap()
                })
                .collect();
            drawn.sort();
            assert_eq!(drawn, ["B", "C", "D"], "round {round}");
        }
        assert_eq!(walk(&mut session, 2), "E -");
    }

    #[test]
    fn a_step_put_into_a_looped_section_comes_round_with_it() {
        let (mut session, music, _) = two_sections();
        session.edit(|plan| {
            plan.loop_section(music).unwrap();
            plan.insert(3, "New", Action::Blank);
        });
        assert_eq!(walk(&mut session, 8), "A B New A B New C -");
    }

    #[test]
    fn a_span_until_a_time_across_a_heading_is_cut_there_and_then() {
        use crate::timing::TimeOfDay;
        let (mut session, [_, b, c, ..]) = three_sections();
        let at = |text: &str| TimeOfDay::parse(text).unwrap();
        session.edit(|plan| {
            let looped = plan.loop_over(b, c).unwrap();
            let sharp = Flow {
                repeat: Repeat::Until(at("10:30")),
                ends: Ending::Sharp,
                ..Flow::default()
            };
            plan.set_flow(looped, sharp).unwrap();
        });
        session.tell_time(at("10:00").since_midnight());
        assert_eq!(walk(&mut session, 4), "A B C B");
        assert!(!session.cut_now());
        session.tell_time(at("10:30").since_midnight());
        assert!(session.cut_now());
        // Going on leaves the span: D is after its last step.
        assert_eq!(walk(&mut session, 1), "D");
        assert!(!session.cut_now());
    }

    #[test]
    fn a_join_across_a_loop_leaves_where_going_on_goes() {
        use crate::timing::TimeOfDay;
        let at = |text: &str| TimeOfDay::parse(text).unwrap().since_midnight();
        // B to D until 10:30, ending with the step that is on; and A
        // joined into B and on to C, half over the loop.
        let walked = |joined: bool| {
            let mut plan = Plan::new("Sabbath");
            let ids = ["A", "B", "C", "D", "E"].map(|name| plan.show(resource(name)));
            let looped = plan.loop_over(ids[1], ids[3]).unwrap();
            let until = Flow {
                repeat: Repeat::Until(TimeOfDay::parse("10:30").unwrap()),
                ends: Ending::Step,
                ..Flow::default()
            };
            plan.set_flow(looped, until).unwrap();
            if joined {
                plan.join_over(ids[0], ids[2]).unwrap();
                assert_eq!(plan.spans().len(), 2, "a span of its own");
            }
            let mut session = Session::new(plan);
            session.tell_time(at("10:00"));
            let before = walk(&mut session, 6);
            // The time comes while C is on: the loop is over with it.
            session.tell_time(at("10:31"));
            format!("{before} {}", walk(&mut session, 2))
        };
        assert_eq!(walked(true), "A B C D B C E -");
        assert_eq!(walked(true), walked(false));
    }
}
