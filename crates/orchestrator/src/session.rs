use std::time::SystemTime;

use chorquest_resource::Resource;
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
#[derive(Debug, Clone)]
pub struct Session {
    plan: Plan,
    cursor: Option<StepId>,
    log: Vec<LogEntry>,
}

impl Session {
    /// A session that hasn't started yet.
    pub fn new(plan: Plan) -> Self {
        Self {
            plan,
            cursor: None,
            log: Vec::new(),
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
            self.cursor = index
                .checked_sub(1)
                .and_then(|i| {
                    self.plan
                        .steps()
                        .get(i.min(self.plan.len().saturating_sub(1)))
                })
                .map(|s| s.id);
        }
        result
    }

    /// Swaps in a different plan and starts over. The log is kept.
    pub fn replace_plan(&mut self, plan: Plan) {
        self.plan = plan;
        self.cursor = None;
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
        self.plan
            .steps()
            .get(self.current_index().map_or(0, |i| i + 1))
    }

    /// Moves to the next step (or the first, if not started). At the end,
    /// stays on the last step and returns `None`.
    #[allow(clippy::should_implement_trait)] // a cursor, not an iterator: it moves both ways
    pub fn next(&mut self) -> Option<&Step> {
        let index = self.current_index().map_or(0, |i| i + 1);
        self.move_to_index(index)
    }

    /// Moves to the previous step. At the first step, stays and returns `None`.
    pub fn prev(&mut self) -> Option<&Step> {
        let index = self.current_index()?.checked_sub(1)?;
        self.move_to_index(index)
    }

    /// Jumps to a step.
    pub fn goto(&mut self, id: StepId) -> Option<&Step> {
        let index = self.plan.position(id)?;
        self.move_to_index(index)
    }

    /// Goes back to "not started".
    pub fn reset(&mut self) {
        self.cursor = None;
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

    /// Inserts a step right after the current one (or first, if not started).
    /// Handy for turning something found live into part of the plan.
    pub fn insert_after_current(&mut self, label: impl Into<String>, action: Action) -> StepId {
        let index = self.current_index().map_or(0, |i| i + 1);
        self.plan.insert(index, label, action)
    }

    /// Everything shown so far, oldest first. Useful for song-usage reports.
    pub fn log(&self) -> &[LogEntry] {
        &self.log
    }

    fn move_to_index(&mut self, index: usize) -> Option<&Step> {
        let step = self.plan.steps().get(index)?;
        self.cursor = Some(step.id);
        Some(step)
    }
}

#[cfg(test)]
mod tests {
    use chorquest_resource::{Kind, Location, ResourceId, SourceId};

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
        s.insert_after_current(
            "Announcement",
            Action::Show {
                resource: resource("Ad"),
            },
        );
        assert_eq!(label(s.next()), Some("Announcement"));
    }
}
