//! The chorquest engine.
//!
//! One [`Engine`] owns the [`Searcher`] (sources and index), the
//! [`Session`] (the plan being run) and the [`Showman`] (the stage), and
//! keeps them consistent: moving through the plan puts things on stage, and
//! everything that goes on stage is logged.
//!
//! The engine is `Send + Sync`. Share it behind an `Arc` between the UI
//! thread, the dashboard and the stage window. Every method returns quickly.
//! The only slow operations are scanning and live searches, which run on
//! worker threads.
//!
//! ```
//! use chorquest_engine::{Engine, Moved, Plan};
//!
//! let engine = Engine::new();
//! engine.load_plan(Plan::new("Sunday service"));
//! let stage = engine.subscribe(); // the stage window listens here
//! assert_eq!(engine.next(), Moved::Nothing); // empty plan
//! # drop(stage);
//! ```

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub use chorquest_orchestrator::{Action, LogEntry, Plan, PlanError, Session, Step, StepId};
pub use chorquest_resource::{
    Capabilities, Hit, Kind, Location, Query, Resource, ResourceId, Source, SourceError, SourceId,
};
pub use chorquest_searcher::{
    Origin, Polled, ReindexReport, SearchError, SearchEvent, SearchStream, Searcher,
};
pub use chorquest_showman::{Playback, Scene, Showman, StageEvent, StageState};

/// Errors from the engine.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// Registering, finding or scanning a source failed.
    #[error(transparent)]
    Search(#[from] SearchError),
    /// Loading or saving a plan failed.
    #[error(transparent)]
    Plan(#[from] PlanError),
    /// No step with that id in the current plan.
    #[error("no step {0} in the plan")]
    UnknownStep(StepId),
}

/// What [`Engine::next`] or [`Engine::prev`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    /// Moved within what's on stage (next stanza, next slide).
    Frame(usize),
    /// Moved to another step of the plan.
    Step(StepId),
    /// Already at the edge; nothing changed.
    Nothing,
}

/// The engine. See the [crate docs](crate).
pub struct Engine {
    searcher: Searcher,
    // Lock order, whenever both are needed: `session`, then `stage`.
    session: Mutex<Session>,
    stage: Mutex<Showman>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// No sources, an empty untitled plan, an empty stage.
    pub fn new() -> Self {
        Self {
            searcher: Searcher::new(),
            session: Mutex::new(Session::new(Plan::new("Untitled"))),
            stage: Mutex::new(Showman::new()),
        }
    }

    // ----- Sources and search -------------------------------------------

    /// Registers a source. Call [`reindex`](Self::reindex) afterwards to scan it.
    pub fn add_source(&self, source: impl Source + 'static) -> Result<(), EngineError> {
        self.add_shared_source(Arc::new(source))
    }

    /// Registers a source you keep a handle to.
    pub fn add_shared_source(&self, source: Arc<dyn Source>) -> Result<(), EngineError> {
        Ok(self.searcher.register(source)?)
    }

    /// The searcher, for anything not wrapped here (unregistering, single-source reindex...).
    pub fn searcher(&self) -> &Searcher {
        &self.searcher
    }

    /// Re-scans every indexed source in parallel. Blocks until done, so call
    /// it from a worker thread in a UI.
    pub fn reindex(&self) -> Vec<ReindexReport> {
        self.searcher.reindex_all()
    }

    /// Searches only the in-memory index. Instant: use it on every keystroke.
    pub fn search_index(&self, query: &Query) -> Vec<Hit> {
        self.searcher.search_index(query)
    }

    /// Searches the index and live sources. See [`Searcher::search`].
    pub fn search(&self, query: Query) -> SearchStream {
        self.searcher.search(query)
    }

    // ----- Plan ----------------------------------------------------------

    /// A copy of the current plan.
    pub fn plan(&self) -> Plan {
        lock(&self.session).plan().clone()
    }

    /// Edits the plan in place, even while running. See [`Session::edit`].
    pub fn edit_plan<R>(&self, f: impl FnOnce(&mut Plan) -> R) -> R {
        lock(&self.session).edit(f)
    }

    /// Replaces the plan and rewinds to the start. The stage is left alone.
    pub fn load_plan(&self, plan: Plan) {
        lock(&self.session).replace_plan(plan);
    }

    /// Loads a plan from a JSON file.
    pub fn open_plan(&self, path: impl AsRef<Path>) -> Result<(), EngineError> {
        self.load_plan(Plan::load(path)?);
        Ok(())
    }

    /// Saves the current plan, including what live slots have collected.
    pub fn save_plan(&self, path: impl AsRef<Path>) -> Result<(), EngineError> {
        Ok(lock(&self.session).plan().save(path)?)
    }

    /// The step the session is at.
    pub fn current_step(&self) -> Option<Step> {
        lock(&self.session).current().cloned()
    }

    /// The step that [`next`](Self::next) would move to.
    pub fn up_next(&self) -> Option<Step> {
        lock(&self.session).peek_next().cloned()
    }

    /// Everything shown so far.
    pub fn log(&self) -> Vec<LogEntry> {
        lock(&self.session).log().to_vec()
    }

    // ----- Running the plan ----------------------------------------------

    /// The "next" button: the next frame of what's on stage if there is one,
    /// otherwise the next step of the plan.
    pub fn next(&self) -> Moved {
        let mut session = lock(&self.session);
        let mut stage = lock(&self.stage);
        if stage.next_frame() {
            return Moved::Frame(current_frame(&stage));
        }
        match session.next().cloned() {
            Some(step) => {
                perform(&mut session, &mut stage, &step);
                Moved::Step(step.id)
            }
            None => Moved::Nothing,
        }
    }

    /// The "back" button: the previous frame, otherwise the previous step.
    pub fn prev(&self) -> Moved {
        let mut session = lock(&self.session);
        let mut stage = lock(&self.stage);
        if stage.prev_frame() {
            return Moved::Frame(current_frame(&stage));
        }
        match session.prev().cloned() {
            Some(step) => {
                perform(&mut session, &mut stage, &step);
                Moved::Step(step.id)
            }
            None => Moved::Nothing,
        }
    }

    /// Jumps to a step and performs it.
    pub fn go_to(&self, id: StepId) -> Result<(), EngineError> {
        let mut session = lock(&self.session);
        let step = session
            .goto(id)
            .cloned()
            .ok_or(EngineError::UnknownStep(id))?;
        perform(&mut session, &mut lock(&self.stage), &step);
        Ok(())
    }

    /// Performs the current step again: back to the plan after showing
    /// things on the fly. Returns the step, if the session has started.
    pub fn resume(&self) -> Option<StepId> {
        let mut session = lock(&self.session);
        let step = session.current().cloned()?;
        perform(&mut session, &mut lock(&self.stage), &step);
        Some(step.id)
    }

    /// Rewinds the plan to "not started". The stage is left alone.
    pub fn rewind(&self) {
        lock(&self.session).reset();
    }

    // ----- Showing things on the fly -------------------------------------

    /// Lines a resource up in preview (dashboard only).
    pub fn preview(&self, resource: Arc<Resource>) {
        lock(&self.stage).preview(resource);
    }

    /// Puts the preview on stage. Returns `false` if the preview was empty.
    pub fn take(&self) -> bool {
        let mut session = lock(&self.session);
        let taken = lock(&self.stage).take();
        match taken {
            Some(resource) => {
                session.record((*resource).clone(), false);
                true
            }
            None => false,
        }
    }

    /// Puts a resource on stage right away, off-plan. Your place in the plan is kept.
    pub fn show_now(&self, resource: Arc<Resource>) {
        let mut session = lock(&self.session);
        lock(&self.stage).show(Arc::clone(&resource));
        session.record((*resource).clone(), false);
    }

    /// Adds a resource to the plan right after the current step, turning a
    /// live find into part of the plan.
    pub fn add_after_current(&self, resource: &Resource) -> StepId {
        lock(&self.session).insert_after_current(
            resource.title.clone(),
            Action::Show {
                resource: resource.clone(),
            },
        )
    }

    // ----- Stage ---------------------------------------------------------

    /// A snapshot of the stage, for a window that just opened.
    pub fn stage(&self) -> StageState {
        lock(&self.stage).state().clone()
    }

    /// Receives every stage change. The stage window renders only `Program`,
    /// `Frame`, `Playback`, `Seek` and `Blackout`; `Preview` is for the dashboard.
    pub fn subscribe(&self) -> Receiver<StageEvent> {
        lock(&self.stage).subscribe()
    }

    /// Direct access to the stage for everything else: blackout, clear,
    /// play/pause/seek, frame jumps, reporting page counts.
    pub fn with_stage<R>(&self, f: impl FnOnce(&mut Showman) -> R) -> R {
        f(&mut lock(&self.stage))
    }
}

/// Makes a step take effect on stage.
fn perform(session: &mut Session, stage: &mut Showman, step: &Step) {
    match &step.action {
        Action::Show { resource } => {
            stage.show(Arc::new(resource.clone()));
            session.record(resource.clone(), true);
        }
        Action::Blank => stage.clear(),
        // Live slots and notes leave the stage as the operator left it.
        Action::Live { .. } | Action::Note => {}
    }
}

fn current_frame(stage: &Showman) -> usize {
    stage.program().map_or(0, |s| s.frame)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_is_shareable_between_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Engine>();
    }
}
