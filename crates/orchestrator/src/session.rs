use std::collections::HashMap;
use std::time::SystemTime;

use lantern_resource::Resource;
use serde::{Deserialize, Serialize};

use crate::{Action, Plan, Step, StepId};

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
}

impl Session {
    /// A session that hasn't started yet.
    pub fn new(plan: Plan) -> Self {
        Self {
            plan,
            cursor: None,
            log: Vec::new(),
            entered: HashMap::new(),
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
        let from = self.current_index().map_or(0, |i| i + 1);
        self.plan.steps().get(self.forward(from)?)
    }

    /// Moves to the next step (or the first, if not started). At the end,
    /// stays on the last step and returns `None`.
    #[allow(clippy::should_implement_trait)] // a cursor, not an iterator: it moves both ways
    pub fn next(&mut self) -> Option<&Step> {
        let index = self.forward(self.current_index().map_or(0, |i| i + 1))?;
        self.move_to_index(index)
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
}
