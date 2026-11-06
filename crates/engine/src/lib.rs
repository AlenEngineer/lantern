//! The lantern engine.
//!
//! One [`Engine`] owns the [`Searcher`] (sources and index), the
//! [`Session`] (the plan being run) and the stage, and keeps them
//! consistent: moving through the plan puts things on stage, and everything
//! that goes on stage is logged. The engine is the only way to change the
//! stage.
//!
//! The engine is `Send + Sync`. Share it behind an `Arc` between the UI
//! thread, the dashboard and the stage window. Every method returns quickly.
//! The only slow operations are scanning and live searches, which run on
//! worker threads.
//!
//! ```
//! use lantern_engine::{Engine, Moved, Plan};
//!
//! let engine = Engine::new();
//! engine.load_plan(Plan::new("Sunday service"));
//! let feed = engine.subscribe(); // the stage window starts from feed.state
//! assert_eq!(engine.next(), Moved::Nothing); // empty plan
//! # drop(feed);
//! ```

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

pub use lantern_orchestrator::{Action, LogEntry, Plan, PlanError, Session, Step, StepId};
pub use lantern_resource::{
    Capabilities, Hit, Kind, Location, Query, Resource, ResourceId, Source, SourceError, SourceId,
};
pub use lantern_searcher::{
    Origin, Polled, ReindexReport, SearchError, SearchEvent, SearchStream, Searcher,
};
use lantern_showman::Showman;
pub use lantern_showman::{
    Media, MediaReport, Playback, Scene, SceneId, StageEvent, StageFeed, StageState, Timed,
};

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
        self.operate(true, |session, stage| {
            if stage.next_frame() {
                return Moved::Frame(current_frame(stage));
            }
            match session.next().cloned() {
                Some(step) => {
                    perform(stage, &step);
                    Moved::Step(step.id)
                }
                None => Moved::Nothing,
            }
        })
    }

    /// The "back" button: the previous frame, otherwise the previous step.
    pub fn prev(&self) -> Moved {
        self.operate(true, |session, stage| {
            if stage.prev_frame() {
                return Moved::Frame(current_frame(stage));
            }
            match session.prev().cloned() {
                Some(step) => {
                    perform(stage, &step);
                    Moved::Step(step.id)
                }
                None => Moved::Nothing,
            }
        })
    }

    /// Jumps to a step and performs it.
    pub fn go_to(&self, id: StepId) -> Result<(), EngineError> {
        self.operate(true, |session, stage| {
            let step = session
                .goto(id)
                .cloned()
                .ok_or(EngineError::UnknownStep(id))?;
            perform(stage, &step);
            Ok(())
        })
    }

    /// Performs the current step again: back to the plan after showing
    /// things on the fly. Returns the step, if the session has started.
    pub fn resume(&self) -> Option<StepId> {
        self.operate(true, |session, stage| {
            let step = session.current().cloned()?;
            perform(stage, &step);
            Some(step.id)
        })
    }

    /// Rewinds the plan to "not started". The stage is left alone.
    pub fn rewind(&self) {
        lock(&self.session).reset();
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

    // ----- The stage, operator side --------------------------------------

    /// Lines a resource up in preview (dashboard only).
    pub fn preview(&self, resource: Arc<Resource>) {
        self.control(|stage| stage.preview(resource));
    }

    /// Empties the preview.
    pub fn clear_preview(&self) {
        self.control(Showman::clear_preview);
    }

    /// Puts the preview on stage. Returns `false` if the preview was empty.
    pub fn take(&self) -> bool {
        self.control(Showman::take)
    }

    /// Puts a resource on stage right away, off-plan. Your place in the plan is kept.
    pub fn show_now(&self, resource: Arc<Resource>) {
        self.control(|stage| stage.show(resource));
    }

    /// Empties the stage. The plan cursor doesn't move.
    pub fn clear(&self) {
        self.control(Showman::clear);
    }

    /// Turns blackout on or off. Pressing it twice stays black.
    pub fn set_blackout(&self, on: bool) {
        self.control(|stage| stage.set_blackout(on));
    }

    /// Flips blackout. Returns the new value.
    pub fn toggle_blackout(&self) -> bool {
        self.control(Showman::toggle_blackout)
    }

    /// Jumps to a stanza, page or slide. Returns `false` if it doesn't exist.
    pub fn goto_frame(&self, frame: usize) -> bool {
        self.control(|stage| stage.goto_frame(frame))
    }

    /// Plays the program; from the end, replays it. Returns `false` if the
    /// program has no timeline.
    pub fn play(&self) -> bool {
        self.control(Showman::play)
    }

    /// Pauses the program. Returns `false` if it has no timeline.
    pub fn pause(&self) -> bool {
        self.control(Showman::pause)
    }

    /// The play/pause button. Returns `false` if the program has no timeline.
    pub fn toggle_playback(&self) -> bool {
        self.control(Showman::toggle_playback)
    }

    /// Jumps the program's timeline, clamped to its duration when known.
    /// Returns `false` if it has no timeline.
    pub fn seek(&self, position: Duration) -> bool {
        self.control(|stage| stage.seek(position))
    }

    // ----- The stage, window side ----------------------------------------

    /// A snapshot of the stage, for a one-off read. To follow the stage, use
    /// [`subscribe`](Self::subscribe).
    pub fn stage(&self) -> StageState {
        lock(&self.stage).state().clone()
    }

    /// The stage now and every change after it, taken under one lock. Start
    /// from `feed.state` and apply each event from `feed.events` with
    /// [`StageState::apply`].
    pub fn subscribe(&self) -> StageFeed {
        lock(&self.stage).subscribe()
    }

    /// What a renderer learned about a scene: page count, timeline,
    /// position, end, failure. Ignored, returning `false`, if the scene has
    /// left the stage or the report doesn't apply to it.
    ///
    /// Takes only the stage lock, so a renderer never waits on the plan
    /// (`save_plan` writes a file under the session lock). A report can't
    /// change which scene is on program, so there is nothing to log.
    pub fn report(&self, scene: SceneId, report: MediaReport) -> bool {
        lock(&self.stage).report(scene, report)
    }

    // ----- The one door --------------------------------------------------

    /// Runs an operator action with both locks, session first. This is the
    /// only place both are taken, so the lock order lives here.
    ///
    /// Logging is derived, not called. If `f` left a scene on program that
    /// wasn't there before (by id, so showing the same resource again
    /// counts), its resource is recorded, as planned or not.
    fn operate<R>(&self, planned: bool, f: impl FnOnce(&mut Session, &mut Showman) -> R) -> R {
        let mut session = lock(&self.session);
        let mut stage = lock(&self.stage);
        let before = stage.program().map(|scene| scene.id);
        let result = f(&mut session, &mut stage);
        if let Some(scene) = stage.program().filter(|scene| Some(scene.id) != before) {
            session.record((*scene.resource).clone(), planned);
        }
        result
    }

    /// An off-plan operator action on the stage alone. Anything it puts on
    /// program is logged as shown live.
    fn control<R>(&self, f: impl FnOnce(&mut Showman) -> R) -> R {
        self.operate(false, |_, stage| f(stage))
    }
}

/// Makes a step take effect on stage.
fn perform(stage: &mut Showman, step: &Step) {
    match &step.action {
        Action::Show { resource } => stage.show(Arc::new(resource.clone())),
        Action::Blank => stage.clear(),
        // Live slots and notes leave the stage as the operator left it.
        Action::Live { .. } | Action::Note => {}
    }
}

fn current_frame(stage: &Showman) -> usize {
    stage.program().map_or(0, Scene::frame)
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
