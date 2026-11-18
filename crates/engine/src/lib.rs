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

pub use lantern_orchestrator::timing::{
    self, Basis, Estimate, Pace, Phase, Schedule, SectionTiming, TimeOfDay, Timing,
};
pub use lantern_orchestrator::{
    Action, BracketError, Carried, Carry, CarryError, DEEPEST, Drawn, Ending, Fade, Flow, FlowOf,
    LogEntry, Looped, Note, NoteId, NoteOn, Over, Place, Plan, PlanError, Repeat, Session, Span,
    SpanError, SpanId, Step, StepId, StepSound,
};
pub use lantern_resource::{
    Capabilities, Hit, Kind, Location, Query, Resource, ResourceId, Source, SourceError, SourceId,
};
pub use lantern_searcher::{
    Bang, Origin, Parsed, Polled, ReindexReport, Scope, SearchError, SearchEvent, SearchStream,
    Searcher, fold,
};
use lantern_showman::Showman;
pub use lantern_showman::{
    FadeOut, FadeTime, Look, LookChange, Media, MediaReport, Playback, Rgb, Scene, SceneId,
    StageEvent, StageFeed, StageState, Timed, fit_frames, fit_stanzas, slide_labels,
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
    /// The step that is on started fading out over this long; the plan
    /// goes on once the stage says it has ([`Engine::faded`]).
    FadingOut(FadeTime),
    /// Already at the edge; nothing changed.
    Nothing,
}

/// The engine. See the [crate docs](crate).
pub struct Engine {
    searcher: Searcher,
    // Lock order, whenever both are needed: `session`, then `stage`.
    session: Mutex<Session>,
    stage: Mutex<Showman>,
    /// The sound the current step is playing over what it shows
    /// ([`StepSound`]), while it is on program. Locked last.
    accompanying: Mutex<Option<SceneId>>,
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
            session: Mutex::new(Session::new(Plan::default())),
            stage: Mutex::new(Showman::new()),
            accompanying: Mutex::new(None),
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

    /// Whether the plan has no steps, asked without copying it.
    pub fn plan_is_empty(&self) -> bool {
        lock(&self.session).plan().is_empty()
    }

    /// Edits the plan in place, even while running. See [`Session::edit`].
    pub fn edit_plan<R>(&self, f: impl FnOnce(&mut Plan) -> R) -> R {
        lock(&self.session).edit(f)
    }

    /// Carries steps of the plan to `to`. See [`Plan::carry`].
    pub fn carry(&self, steps: &[StepId], to: Place, how: Carry) -> Result<Carried, CarryError> {
        self.edit_plan(|plan| plan.carry(steps, to, how))
    }

    /// Puts a new section round the rows from `a` to `b`. See
    /// [`Plan::bracket_over`].
    pub fn bracket_over(
        &self,
        a: StepId,
        b: StepId,
        label: impl Into<String>,
    ) -> Result<StepId, BracketError> {
        self.edit_plan(|plan| plan.bracket_over(a, b, label))
    }

    /// Takes a section away and leaves its steps where they are. See
    /// [`Plan::unbracket`].
    pub fn unbracket(&self, section: StepId) -> bool {
        self.edit_plan(|plan| plan.unbracket(section))
    }

    /// Has a section end at a row, or run on. See [`Plan::reach_section`].
    pub fn reach_section(&self, section: StepId, last: Option<StepId>) -> Result<(), BracketError> {
        self.edit_plan(|plan| plan.reach_section(section, last))
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

    /// Whether the current step goes on by itself when done: a join
    /// holds it ([`Plan::goes_on`]).
    pub fn goes_on(&self) -> bool {
        let session = lock(&self.session);
        session
            .current_index()
            .is_some_and(|at| session.plan().goes_on(at))
    }

    /// When the service began: when its first step was reached.
    pub fn started(&self) -> Option<std::time::SystemTime> {
        lock(&self.session).started()
    }

    /// Which time round a repeating step, span or plan is on: 0 the
    /// first time. A section's heading says it for the span attached to
    /// the section.
    pub fn round_of(&self, of: impl Into<FlowOf>) -> u16 {
        lock(&self.session).round_of(of)
    }

    /// Whether the service has reached step `id` yet.
    pub fn reached(&self, id: StepId) -> bool {
        lock(&self.session).entered(id).is_some()
    }

    /// The steps that have come already this round of each shuffled span
    /// the step that is on is in.
    pub fn drawn_before(&self) -> Vec<StepId> {
        lock(&self.session).drawn_before()
    }

    /// What going on from the step that is on goes back round: the step
    /// itself, a span or the plan. `None` when it goes forward.
    pub fn follows_back(&self) -> Option<FlowOf> {
        lock(&self.session).follows_back()
    }

    /// Which time round `of` is on, from 0, and of how many: `None` for
    /// one with no number to it.
    pub fn rounds(&self, of: impl Into<FlowOf>) -> (u16, Option<u16>) {
        lock(&self.session).rounds(of)
    }

    /// Whether what is on should be cut off now: it is in something
    /// that comes again until a time of day and ends there and then, and
    /// the time has come. Going on then leaves it, fading it out first
    /// where its step fades out; while it fades, it is being cut.
    pub fn cut_now(&self) -> bool {
        let session = lock(&self.session);
        session.cut_now() && lock(&self.stage).state().fading_out.is_none()
    }

    /// The time of day, as time since midnight, for a repeat that ends
    /// at one.
    pub fn tell_time(&self, since_midnight: Duration) {
        lock(&self.session).tell_time(since_midnight);
    }

    /// The step that [`next`](Self::next) would move to.
    pub fn up_next(&self) -> Option<Step> {
        lock(&self.session).peek_next().cloned()
    }

    /// Everything shown so far.
    pub fn log(&self) -> Vec<LogEntry> {
        lock(&self.session).log().to_vec()
    }

    /// How each section of the plan is keeping to its budget, now. `loaded`
    /// gives a step's media length once the app has loaded it. While the
    /// current step's video plays, what's left of it is exact.
    pub fn timing(&self, loaded: &dyn Fn(&Step) -> Option<Duration>) -> Timing {
        let session = lock(&self.session);
        let current = session.current().cloned();
        let current_left = current.as_ref().and_then(|step| {
            let Action::Show { resource } = &step.action else {
                return None;
            };
            let stage = lock(&self.stage);
            let scene = stage
                .program()
                .filter(|scene| scene.resource.id == resource.id)?;
            let timed = scene.timed()?;
            Some(timed.duration?.saturating_sub(timed.position))
        });
        let entered = |id| session.entered(id);
        timing::timing(
            session.plan(),
            &timing::Clock {
                now: std::time::SystemTime::now(),
                current: current.map(|step| step.id),
                entered: &entered,
                current_left,
                loaded,
                of_day: session.time_of_day(),
            },
        )
    }

    // ----- Running the plan ----------------------------------------------

    /// The "next" button: the next frame of what's on stage if there is one,
    /// otherwise the next step of the plan. Under a sound, the frames are
    /// those of the picture it plays over.
    ///
    /// A step that fades out ([`Fade::fade_out`]) and still has something
    /// on stage fades it out first: the plan goes on once the stage says
    /// it has faded ([`faded`](Self::faded)). Pressed again while it
    /// fades, it goes on at once.
    pub fn next(&self) -> Moved {
        self.operate(true, |session, stage| {
            if stage.next_frame() {
                return Moved::Frame(current_frame(stage));
            }
            if stage.state().fading_out.is_none()
                && let Some(fade) = session
                    .current()
                    .and_then(|step| how_it_fades_out(step, stage.state()))
            {
                stage.fade_out(fade);
                return Moved::FadingOut(fade.over);
            }
            go_on(session, stage)
        })
    }

    /// The stage has faded out what was on, `scene` on program, as it was
    /// told to: that comes off, and the plan goes on from the step that
    /// put it there. Where the operator went to another step meanwhile,
    /// one that leaves the stage as it is, the plan stays there. Ignored,
    /// returning `false`, when that scene isn't on program or isn't
    /// fading out any more: something else was put on.
    pub fn faded(&self, scene: SceneId) -> bool {
        self.operate(true, |session, stage| {
            let state = stage.state();
            let due = state.fading_out.is_some()
                && stage.program().is_some_and(|program| program.id == scene);
            if !due {
                return false;
            }
            let from_here = session.current().is_some_and(|step| {
                [state.program.as_ref(), state.under.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(|scene| of_step(step, scene))
            });
            match from_here {
                true => {
                    go_on(session, stage);
                }
                false => take_off(stage),
            }
            true
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

    /// [`go_to`](Self::go_to), with what the step shows starting at
    /// `frame`: back to where a service was when it was interrupted.
    pub fn go_to_at(&self, id: StepId, frame: usize) -> Result<(), EngineError> {
        self.operate(true, |session, stage| {
            let step = session
                .goto(id)
                .cloned()
                .ok_or(EngineError::UnknownStep(id))?;
            match &step.action {
                Action::Show { resource } => {
                    stage.show_fading_in(Arc::new(resource.clone()), frame, step.fade.fade_in);
                }
                _ => perform(stage, &step),
            }
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

    /// Adds a step to the plan right after `after`, or, without one, after
    /// the current step (at the end before the service starts).
    pub fn add(&self, after: Option<StepId>, label: impl Into<String>, action: Action) -> StepId {
        lock(&self.session).insert_after(after, label, action)
    }

    /// Adds a step showing `resource`, titled after it. See [`add`](Self::add).
    pub fn add_resource(&self, after: Option<StepId>, resource: &Resource) -> StepId {
        let action = Action::Show {
            resource: resource.clone(),
        };
        self.add(after, resource.title.clone(), action)
    }

    // ----- The stage, operator side --------------------------------------

    /// Lines a resource up in preview (dashboard only).
    pub fn preview(&self, resource: Arc<Resource>) {
        self.control(|stage| stage.preview(resource));
    }

    /// Lines a resource up in preview at one of its frames.
    pub fn preview_at(&self, resource: Arc<Resource>, frame: usize) {
        self.control(|stage| stage.preview_at(resource, frame));
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

    /// [`show_now`](Self::show_now), starting at `frame` of something
    /// shown in frames.
    pub fn show_now_at(&self, resource: Arc<Resource>, frame: usize) {
        self.control(|stage| stage.show_at(resource, frame));
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
    ///
    /// When the current step's video ends and the step goes on by itself
    /// ([`Plan::goes_on`]), the plan moves to its next step, with no
    /// fade: it is over.
    pub fn report(&self, scene: SceneId, report: MediaReport) -> bool {
        let ended = report == MediaReport::Ended;
        let applied = lock(&self.stage).report(scene, report);
        if applied && ended && self.goes_on_after(scene) {
            self.next();
        }
        applied
    }

    /// Whether `scene` is on program for the current step, and that step
    /// goes on by itself.
    fn goes_on_after(&self, scene: SceneId) -> bool {
        let Some(step) = self.current_step().filter(|_| self.goes_on()) else {
            return false;
        };
        let Action::Show { resource } = &step.action else {
            return false;
        };
        let stage = lock(&self.stage);
        stage
            .program()
            .is_some_and(|program| program.id == scene && program.resource.id == resource.id)
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
        accompany(&session, &mut stage, &mut lock(&self.accompanying));
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
        Action::Show { resource } => {
            stage.show_fading_in(Arc::new(resource.clone()), 0, step.fade.fade_in);
        }
        Action::Blank => stage.clear(),
        // Live slots and notes leave the stage as the operator left it, and
        // the session never stops on a section marker.
        Action::Live { .. } | Action::Note | Action::Section { .. } => {}
    }
}

/// Moves the session to its next step and performs it. What was fading
/// out comes off first, so a step that leaves the stage as it is (a
/// note, a live slot) doesn't bring it back.
fn go_on(session: &mut Session, stage: &mut Showman) -> Moved {
    take_off(stage);
    match session.next().cloned() {
        Some(step) => {
            perform(stage, &step);
            Moved::Step(step.id)
        }
        None => Moved::Nothing,
    }
}

/// Takes off what was fading out, if anything was: the picture with what
/// plays over it, or the sound alone, leaving the picture under it.
fn take_off(stage: &mut Showman) {
    match stage.state().fading_out.map(|fade| fade.picture) {
        Some(true) => stage.clear(),
        Some(false) => {
            stage.lift_sound();
        }
        None => {}
    }
}

/// Whether `scene` is what `step` shows, or the sound it plays with that.
fn of_step(step: &Step, scene: &Scene) -> bool {
    match &step.action {
        Action::Show { resource } => {
            scene.resource.id == resource.id
                || step
                    .sound
                    .as_ref()
                    .is_some_and(|sound| sound.resource.id == scene.resource.id)
        }
        _ => false,
    }
}

/// How what `step` has on stage fades out as the plan goes on from it:
/// its picture with what plays over it, or its sound alone over another
/// step's picture. `None` where it goes at once: it doesn't fade out,
/// nothing of it is on stage, or what is on has ended or failed.
fn how_it_fades_out(step: &Step, stage: &StageState) -> Option<FadeOut> {
    let over = step.fade.fade_out?;
    let Action::Show { resource } = &step.action else {
        return None;
    };
    let picture = stage
        .picture()
        .is_some_and(|seen| seen.resource.id == resource.id && still_on(seen));
    let sound = stage
        .program
        .as_ref()
        .is_some_and(|on| on.sounds_only() && of_step(step, on) && still_on(on));
    (picture || sound).then_some(FadeOut { over, picture })
}

/// Whether a scene still shows or plays something: not one that has
/// failed, nor a timeline that has ended.
fn still_on(scene: &Scene) -> bool {
    match &scene.media {
        Media::Timed(timed) => timed.playback != Playback::Ended,
        Media::Failed { .. } => false,
        Media::Frames { .. } => true,
    }
}

/// Keeps the current step's sound ([`StepSound`]) with what the step
/// shows, after anything the operator did: it goes on over the step's
/// picture once that is at the slide the sound starts from, and comes off,
/// leaving the picture, when the picture goes back before that slide or
/// the plan moves to a step that leaves the stage as it is.
///
/// `playing` is the sound this put on program. One the operator replaced
/// or cleared is forgotten, and isn't started again over the same slides.
fn accompany(session: &Session, stage: &mut Showman, playing: &mut Option<SceneId>) {
    let program = stage.program().map(|scene| scene.id);
    if playing.is_some() && *playing != program {
        *playing = None;
    }
    let wanted = session.current().and_then(|step| {
        let sound = step.sound.as_ref()?;
        let Action::Show { resource } = &step.action else {
            return None;
        };
        let picture = stage.state().picture()?;
        let due = picture.resource.id == resource.id
            && picture.timed().is_none()
            && picture.frame() >= sound.from;
        due.then(|| (sound.resource.clone(), step.fade.fade_in))
    });
    match (wanted, *playing) {
        // Over the picture itself: not over a sound the operator put on.
        (Some((sound, fade_in)), None) if stage.program().is_some_and(|on| !on.sounds_only()) => {
            stage.show_fading_in(Arc::new(sound), 0, fade_in);
            *playing = stage.program().map(|scene| scene.id);
        }
        (None, Some(_)) => {
            stage.lift_sound();
            *playing = None;
        }
        _ => {}
    }
}

fn current_frame(stage: &Showman) -> usize {
    stage.state().picture().map_or(0, Scene::frame)
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
