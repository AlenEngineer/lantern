//! The lantern desktop app: an operator dashboard and a fullscreen stage
//! window for the projector, both following one [`Engine`].
//!
//! It reads its media folders and Bibles from a config file (see
//! [`config::load`]) and indexes them in the background.
//!
//! Set `LANTERN_SCRIPT=<file>` to drive the engine from a script and exit
//! after its last line (see [`script`]). `LANTERN_SHOT=<program>` is run
//! for each `shot NAME` in it.

mod config;
mod dashboard;
mod keys;
mod length;
mod library;
mod look;
mod palette;
mod plan_pane;
mod plans;
mod prepare;
mod script;
mod search;
mod settings;
mod stage;
mod youtube;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use iced::futures::{SinkExt, Stream, StreamExt};
use iced::widget::{Space, container, image, operation, scrollable, stack, text};
use iced::{Color, ContentFit, Element, Fill, Subscription, Task, window};
use iced_video_player::VideoPlayer;
use lantern_engine::{
    Action, Engine, Media, MediaReport, Plan, Playback, ResourceId, SceneId, SearchEvent,
    SearchStream, StageEvent, StageFeed, StageState, StepId,
};

use crate::config::Config;
use crate::dashboard::{Mode, NAME_BOX, RESULTS, SEARCH_BOX};
use crate::keys::Op;
use crate::library::Library;
use crate::palette::Palette;
use crate::plans::{Plans, Store, Switch};
use crate::prepare::View;
use crate::script::{Script, Ui};
use crate::search::{Live, Search};
use crate::stage::{Content, Loaded, Stage};
use crate::youtube::{Downloads, YoutubeVideo};

/// How long typing must pause before the live sources are asked.
const LIVE_SEARCH_PAUSE: Duration = Duration::from_millis(250);

/// The Wayland app id of the audience window. A Hyprland rule matches it to
/// send the window to the projector.
const STAGE_APP_ID: &str = "lantern-stage";
const DASHBOARD_APP_ID: &str = "lantern-dashboard";

fn main() -> ExitCode {
    log("starting");
    let script = match load_script() {
        Ok(script) => script,
        Err(err) => {
            log(format!("LANTERN_SCRIPT: {err}"));
            return ExitCode::FAILURE;
        }
    };
    let (config, config_status) = config::load(config::env);
    log(config_status.to_string());
    let config_status = Arc::new(config_status);
    let engine = Shared(Arc::new(Engine::new()));
    let library = Shared(Library::start(engine.0.clone(), config.clone()));
    let downloads = Shared(Downloads::new(stage::cache_dir().join("youtube")));
    let store = Store::new(&config::xdg_dir(
        config::env,
        "XDG_DATA_HOME",
        ".local/share",
    ));
    let result = iced::daemon(
        move || {
            App::boot(
                engine.clone(),
                library.clone(),
                config_status.clone(),
                config.clone(),
                downloads.clone(),
                store.clone(),
                script.clone(),
            )
        },
        App::update,
        App::view,
    )
    .title(App::title)
    .theme(|app: &App, _| look::theme(app.palette))
    .default_font(look::UI)
    .subscription(App::subscription)
    .run();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log(format!("iced: {err}"));
            ExitCode::FAILURE
        }
    }
}

fn load_script() -> Result<Option<Script>, String> {
    let Some(path) = std::env::var_os("LANTERN_SCRIPT") else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    let source =
        std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let shooter = std::env::var_os("LANTERN_SHOT").map(PathBuf::from);
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    Script::parse(&source, dir, shooter)
        .map(Some)
        .map_err(|err| format!("{}: {err}", path.display()))
}

/// Writes one timestamped line to stderr, which the e2e harness keeps as `app.log`.
fn log(message: impl AsRef<str>) {
    static START: OnceLock<Instant> = OnceLock::new();
    let elapsed = START.get_or_init(Instant::now).elapsed().as_secs_f64();
    eprintln!("[{elapsed:8.3}] {}", message.as_ref());
}

/// Shared state, hashed by identity so it can key a [`Subscription`].
struct Shared<T>(Arc<T>);

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Shared(self.0.clone())
    }
}

impl<T> Hash for Shared<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

#[derive(Debug, Clone)]
enum Message {
    /// The stage as it was when the feed started. Always the feed's first message.
    Feed(StageState),
    Stage(StageEvent),
    Loaded(Arc<Loaded>),
    /// The program's video reached its end.
    Ended(SceneId),
    /// Time to report the playing video's position.
    Tick,
    /// Time to redraw download or indexing progress.
    Progress,
    /// The script performed a command. The plan may have changed, which the
    /// stage feed doesn't announce.
    Acted,
    ScriptDone,
    /// A dashboard command from the script.
    Script(Ui),
    Closed(window::Id),
    /// The search box changed.
    Query(String),
    /// The pause after this edit of the query is over.
    Paused(u64),
    /// An event from the live search with this ticket.
    Searched(u64, Arc<SearchEvent>),
    /// Every source answered the live search with this ticket.
    SearchDone(u64),
    /// A result was clicked.
    Pick(ResourceId),
    /// A bang was clicked: start a query with it.
    Bang(String),
    /// A key, button or script press. `only_if_empty`: it only counts while
    /// the search box is empty (the arrows, which otherwise move the cursor).
    Key {
        op: Op,
        only_if_empty: bool,
    },
    /// Text typed while the search box didn't have focus: it goes there.
    Type(String),
    /// Time to see whether the colours file changed.
    CheckPalette,
    /// A plan step was clicked: it becomes the selection, where new steps
    /// go, and in the run view it's lined up in the preview.
    Select(StepId),
    Remove(StepId),
    /// Adds a section, live slot, black screen or note after the selection.
    AddStep(prepare::NewStep),
    /// The plan's new name, typed in the prepare view.
    PlanTitle(String),
    /// A step's or section's new name.
    Rename(StepId, String),
    /// What was typed in a step's length or a section's budget.
    Length(StepId, String),
    /// Moves a step up (-1) or down (1).
    Move(StepId, isize),
    /// A change in the settings view.
    Settings(settings::Edit),
    /// Writes the settings to the config file and applies them.
    SaveSettings,
    /// A saved plan in the open list was clicked.
    OpenPlan(PathBuf),
    /// The operator agreed to lose the unsaved plan.
    DiscardPlan,
    /// Back to the steps from the name editor, the open list or the question.
    ClosePlanMode,
    EditPlanName,
    PlanName(String),
    SubmitPlanName,
    /// The timeline is being dragged to this many seconds.
    Scrub(f64),
    /// The timeline was let go.
    Scrubbed,
}

struct App {
    engine: Shared<Engine>,
    library: Shared<Library>,
    /// Which config file was read, or why none was.
    config_status: Arc<config::Status>,
    /// The config in force, for the settings view to start from.
    config: Config,
    /// The settings being edited, while the view is open.
    settings: Option<settings::Draft>,
    home: Option<PathBuf>,
    downloads: Shared<Downloads>,
    plans: Plans,
    palette: Palette,
    /// The roles file the palette follows, if the config names one.
    colors: Option<palette::Source>,
    /// Why the roles file couldn't be read, while it can't.
    palette_error: Option<String>,
    script: Option<Script>,
    stage_window: window::Id,
    /// The stage window's mirror, built only from the engine's feed.
    stage: StageState,
    media: Stage,
    search: Search,
    mode: Mode,
    /// Run or prepare.
    view: View,
    /// The selected plan step: where new steps go, and in the run view
    /// what Enter puts on stage.
    selected: Option<StepId>,
    /// Lengths being typed that don't read as one yet ("4:"), by step.
    drafts: HashMap<StepId, String>,
    /// Where the timeline is being dragged, until it is let go.
    scrub: Option<f64>,
}

impl App {
    fn boot(
        engine: Shared<Engine>,
        library: Shared<Library>,
        config_status: Arc<config::Status>,
        config: Config,
        downloads: Shared<Downloads>,
        store: Store,
        script: Option<Script>,
    ) -> (App, Task<Message>) {
        let (dashboard_window, open_dashboard) = window::open(window::Settings {
            size: iced::Size::new(1280.0, 720.0),
            platform_specific: platform(DASHBOARD_APP_ID),
            ..window::Settings::default()
        });
        let (stage_window, open_stage) = window::open(window::Settings {
            fullscreen: true,
            decorations: false,
            platform_specific: platform(STAGE_APP_ID),
            ..window::Settings::default()
        });
        // The stage feed's first message preloads the reopened plan.
        let plans = Plans::start(store, &engine.0);
        let mut app = App {
            media: Stage::new(downloads.0.clone()),
            plans,
            engine,
            library,
            config_status,
            palette: Palette::default(),
            colors: config.colors.clone().map(palette::Source::new),
            config,
            settings: None,
            home: config::env("HOME").map(PathBuf::from),
            palette_error: None,
            downloads,
            script,
            stage_window,
            stage: StageState::default(),
            search: Search::default(),
            mode: Mode::default(),
            view: View::default(),
            selected: None,
            drafts: HashMap::new(),
            scrub: None,
        };
        app.check_palette();
        // Dashboard first: on a single monitor, Hyprland drops a fullscreen
        // window back to tiled when another window maps on its workspace.
        // Then the keyboard goes back to the dashboard, into the search box.
        let open = open_dashboard
            .discard()
            .chain(open_stage.discard())
            .chain(window::gain_focus(dashboard_window))
            .chain(operation::focus(SEARCH_BOX));
        (app, open)
    }

    fn title(&self, window: window::Id) -> String {
        if window == self.stage_window {
            "lantern stage".into()
        } else {
            "lantern".into()
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Feed(state) => {
                self.stage = state;
                return self.program_changed();
            }
            Message::Stage(event) => return self.stage_event(&event),
            Message::Loaded(loaded) => self.media.loaded(&loaded, &self.stage, &self.engine.0),
            Message::Ended(scene) => {
                self.engine.0.report(scene, MediaReport::Ended);
            }
            Message::Tick => {
                if let Some((scene, video)) = self.media.video() {
                    let position = video.position();
                    self.engine.0.report(scene, MediaReport::Position(position));
                }
            }
            Message::Progress => {}
            Message::Acted => return self.preload(),
            Message::ScriptDone => {
                log("script finished, exiting");
                return iced::exit();
            }
            Message::Script(ui) => {
                log(format!("dashboard: {ui:?}"));
                return match ui {
                    Ui::Type(query) => {
                        // A script types a whole new search, chip and all.
                        self.search.clear(&self.engine.0);
                        self.query(query)
                    }
                    Ui::Select(n) => match self.search.pick_nth(n) {
                        Some(resource) => {
                            self.engine.0.preview(resource);
                            self.scroll_to_pick()
                        }
                        None => {
                            log(format!("dashboard: no result {}", n + 1));
                            Task::none()
                        }
                    },
                    Ui::Step(n) => match self.engine.0.plan().steps().get(n) {
                        Some(step) => self.update(Message::Select(step.id)),
                        None => {
                            log(format!("dashboard: no plan step {}", n + 1));
                            Task::none()
                        }
                    },
                    Ui::Press(op) => self.op(op),
                    Ui::Open(name) => {
                        let opened = self.plans.open_named(&self.engine.0, &name);
                        self.preload_if(opened)
                    }
                    Ui::Rename(name) => Task::batch([
                        self.update(Message::EditPlanName),
                        self.update(Message::PlanName(name)),
                        self.update(Message::SubmitPlanName),
                    ]),
                };
            }
            Message::Query(query) => return self.query(query),
            Message::Paused(edit) => {
                if let Some(stream) = self.search.start_live(edit, &self.engine.0) {
                    return live_search(stream);
                }
            }
            Message::Searched(ticket, event) => {
                self.search.live_event(ticket, &event, &self.engine.0);
            }
            Message::SearchDone(ticket) => self.search.live_done(ticket),
            Message::Pick(id) => {
                if let Some(resource) = self.search.pick(&id) {
                    self.engine.0.preview(resource);
                }
            }
            Message::Bang(bang) => return self.complete(format!("!{bang} ")),
            Message::Key { op, only_if_empty } => {
                if !(only_if_empty && !self.search.query.is_empty()) {
                    return self.op(op);
                }
            }
            // Typing goes to the search box, which settings cover.
            Message::Type(_) if self.settings.is_some() => {}
            Message::Type(typed) => {
                let query = format!("{}{typed}", self.search.query);
                return self.complete(query);
            }
            Message::CheckPalette => self.check_palette(),
            Message::Settings(edit) => {
                if let Some(draft) = &mut self.settings {
                    draft.edit(edit);
                }
            }
            Message::SaveSettings => self.save_settings(),
            Message::Select(step) => {
                self.selected = Some(step);
                if let Some(Action::Show { resource }) = self.cued().map(|s| s.action) {
                    self.search.unpick();
                    self.engine.0.preview(Arc::new(resource));
                } else if self.view == View::Run {
                    self.engine.0.clear_preview();
                }
                return self.preload();
            }
            Message::Remove(step) => {
                if self.selected == Some(step) {
                    self.selected = None;
                }
                self.engine.0.edit_plan(|plan| plan.remove(step));
                return self.preload();
            }
            Message::AddStep(new) => {
                let (label, action) = new.step();
                self.selected = Some(self.engine.0.add(self.selected, label, action));
            }
            Message::PlanTitle(title) => self.engine.0.edit_plan(|plan| plan.title = title),
            Message::Rename(id, label) => self.engine.0.edit_plan(|plan| {
                if let Some(step) = plan.get_mut(id) {
                    step.label = label;
                }
            }),
            Message::Length(id, typed) => {
                if let Some(value) = length::parse(&typed) {
                    self.engine.0.edit_plan(|plan| set_length(plan, id, value));
                }
                self.drafts.insert(id, typed);
            }
            Message::Move(id, by) => {
                self.engine.0.edit_plan(|plan| {
                    if let Some(at) = plan.position(id) {
                        plan.move_to(id, at.saturating_add_signed(by));
                    }
                });
                return self.preload();
            }
            Message::OpenPlan(path) => {
                let opened = self.plans.switch(&self.engine.0, Switch::Open(path));
                return self.preload_if(opened);
            }
            Message::DiscardPlan => {
                let replaced = self.plans.discard(&self.engine.0);
                return self.preload_if(replaced);
            }
            Message::ClosePlanMode => {
                self.plans.cancel();
                return operation::focus(SEARCH_BOX);
            }
            Message::EditPlanName => {
                self.plans.edit_name(&self.engine.0);
                return Task::batch([operation::focus(NAME_BOX), operation::select_all(NAME_BOX)]);
            }
            Message::PlanName(text) => self.plans.type_name(text),
            Message::SubmitPlanName => {
                self.plans.submit_name(&self.engine.0);
                return operation::focus(SEARCH_BOX);
            }
            Message::Scrub(seconds) => self.scrub = Some(seconds),
            Message::Scrubbed => {
                if let Some(seconds) = self.scrub.take() {
                    self.engine.0.seek(Duration::from_secs_f64(seconds));
                }
            }
            Message::Closed(window) => {
                log(format!("window {window:?} closed, exiting"));
                return iced::exit();
            }
        }
        Task::none()
    }

    /// Index results now, the live sources once typing pauses.
    fn query(&mut self, query: String) -> Task<Message> {
        if !query.trim().is_empty() {
            self.mode = Mode::Live;
            // Running, a new search takes over Enter from the selected step.
            if self.view == View::Run {
                self.selected = None;
            }
        }
        let had_bang = self.search.bang().is_some();
        let edit = self.search.edit(query, &self.engine.0);
        let searched = self.searched(edit);
        // A bang just became the chip: the box got shorter under the cursor.
        if !had_bang && self.search.bang().is_some() {
            return Task::batch([searched, operation::move_cursor_to_end(SEARCH_BOX)]);
        }
        searched
    }

    /// After an edit of the search: the live search once typing pauses,
    /// and the results scrolled to the top.
    fn searched(&mut self, edit: Option<u64>) -> Task<Message> {
        let Some(edit) = edit else {
            return Task::none();
        };
        let pause = Task::perform(after(LIVE_SEARCH_PAUSE), move |()| Message::Paused(edit));
        Task::batch([
            pause,
            operation::snap_to(RESULTS, scrollable::RelativeOffset::START),
        ])
    }

    fn op(&mut self, op: Op) -> Task<Message> {
        let engine = self.engine.0.clone();
        match op {
            Op::Enter => {
                if self.search.completion(&engine).is_some() {
                    return self.op(Op::Complete);
                }
                if let Some(stream) = self.search.start_on_enter(&engine) {
                    return live_search(stream);
                }
                // Preparing, Enter adds the result to the plan; running, it
                // shows the plan step lined up in the preview.
                match self.view {
                    View::Prepare => return self.op(Op::AddToPlan),
                    View::Run if self.cued().is_some() => return self.op(Op::Take),
                    View::Run => {}
                }
                if self.search.picked().is_none() {
                    if let Some(first) = self.search.pick_nth(0) {
                        engine.preview(first);
                        return self.scroll_to_pick();
                    }
                    // Still searching: there's nothing new to take yet.
                    if self.search.live != Live::Idle {
                        return Task::none();
                    }
                }
                return self.op(Op::Take);
            }
            Op::Take => {
                // The selected plan step, if it's what's lined up: going to
                // it keeps the plan, the timer and PgDn in step.
                let preview = engine.stage().preview;
                let cued = self.cued().filter(|step| match (&step.action, &preview) {
                    (Action::Show { resource }, Some(scene)) => resource.id == scene.resource.id,
                    (_, Some(_)) => false,
                    (action, None) => !matches!(action, Action::Show { .. }),
                });
                if let Some(step) = cued {
                    self.mode = Mode::Plan;
                    self.selected = None;
                    if let Err(err) = engine.go_to(step.id) {
                        log(format!("dashboard: {err}"));
                    }
                    return self.preload();
                }
                if engine.take() {
                    self.search.unpick();
                }
            }
            Op::AddToPlan => {
                let resource = match self.view {
                    View::Prepare => self
                        .search
                        .picked()
                        .cloned()
                        .or_else(|| self.search.first()),
                    View::Run => engine.stage().preview.map(|scene| scene.resource.clone()),
                };
                if let Some(resource) = resource {
                    self.selected = Some(engine.add_resource(self.selected, &resource));
                    return self.preload();
                }
            }
            Op::Next | Op::Prev => {
                self.mode = Mode::Plan;
                if op == Op::Next {
                    engine.next();
                } else {
                    engine.prev();
                }
                return self.preload();
            }
            Op::PlayPause => {
                engine.toggle_playback();
            }
            Op::Blackout => {
                engine.toggle_blackout();
            }
            Op::Clear => engine.clear(),
            Op::PickNext | Op::PickPrev => {
                if let Some(resource) = self.search.step(op == Op::PickNext) {
                    engine.preview(resource);
                    return self.scroll_to_pick();
                }
            }
            Op::ClearSearch if self.settings.is_some() => return self.op(Op::Settings),
            Op::ClearSearch => {
                self.search.clear(&engine);
                return operation::focus(SEARCH_BOX);
            }
            Op::Save if self.settings.is_some() => self.save_settings(),
            Op::Settings => {
                self.settings = match self.settings {
                    Some(_) => None,
                    None => Some(settings::Draft::of(&self.config, self.home.as_deref())),
                };
                if self.settings.is_none() {
                    return operation::focus(SEARCH_BOX);
                }
            }
            Op::DropBang => {
                let edit = self.search.drop_bang(&engine);
                return self.searched(edit);
            }
            Op::Save => {
                self.plans.save(&engine);
            }
            Op::Open => self.plans.browse(),
            Op::New => {
                let replaced = self.plans.switch(&engine, Switch::New);
                return self.preload_if(replaced);
            }
            Op::ToggleMode => self.mode = self.mode.toggled(),
            Op::ToggleView => {
                self.view = self.view.toggled();
                self.drafts.clear();
                return operation::focus(SEARCH_BOX);
            }
            Op::MoveUp | Op::MoveDown => {
                if let (View::Prepare, Some(step)) = (self.view, self.selected) {
                    let by = if op == Op::MoveUp { -1 } else { 1 };
                    return self.update(Message::Move(step, by));
                }
            }
            Op::Complete => {
                if let Some(text) = self.search.completion(&engine) {
                    return self.complete(text);
                }
            }
        }
        Task::none()
    }

    /// Writes the settings to the config file and applies them: the library
    /// swaps its sources and the colours follow the new file. What happened
    /// shows under the settings.
    fn save_settings(&mut self) {
        let Some(draft) = &mut self.settings else {
            return;
        };
        let path = self.config_status.path().to_path_buf();
        let home = self.home.as_deref();
        let dir = path.parent().unwrap_or(std::path::Path::new("."));
        let saved = draft
            .config(dir, home)
            .and_then(|config| config.save(&path, home).map(|()| config));
        match saved {
            Ok(config) => {
                draft.notice = Some(Ok(settings::saved(&path, home)));
                log(format!("settings: saved {}", path.display()));
                self.library.0.apply(config.clone());
                if config.colors != self.config.colors {
                    self.colors = config.colors.clone().map(palette::Source::new);
                    if self.colors.is_none() {
                        self.palette = Palette::default();
                        self.palette_error = None;
                    }
                    self.check_palette();
                }
                self.config = config;
                self.config_status = Arc::new(config::Status::Read(path));
            }
            Err(err) => draft.notice = Some(Err(err)),
        }
    }

    /// Rereads the colours file if it changed. A file that can't be read
    /// keeps the colours the app has, and says why.
    fn check_palette(&mut self) {
        let Some(changed) = self.colors.as_mut().and_then(palette::Source::changed) else {
            return;
        };
        match changed {
            Ok(palette) => {
                self.palette = palette;
                self.palette_error = None;
            }
            Err(err) => {
                log(format!("colors: {err}"));
                self.palette_error = Some(format!("Colours: {err}"));
            }
        }
    }

    /// Puts `text` in the search box, with the cursor after it, and searches it.
    fn complete(&mut self, text: String) -> Task<Message> {
        Task::batch([
            self.query(text),
            operation::focus(SEARCH_BOX),
            operation::move_cursor_to_end(SEARCH_BOX),
        ])
    }

    /// Preloads for a plan that replaced the last one.
    fn preload_if(&mut self, replaced: bool) -> Task<Message> {
        if replaced {
            self.preload()
        } else {
            Task::none()
        }
    }

    fn scroll_to_pick(&self) -> Task<Message> {
        match self.search.picked_offset() {
            Some(y) => operation::snap_to(
                RESULTS,
                scrollable::RelativeOffset {
                    x: None,
                    y: Some(y),
                },
            ),
            None => Task::none(),
        }
    }

    fn stage_event(&mut self, event: &StageEvent) -> Task<Message> {
        if !self.stage.apply(event) {
            return Task::none();
        }
        let engine = &self.engine.0;
        match event {
            StageEvent::Program { .. } => return self.program_changed(),
            StageEvent::Preview { .. } => return self.preload(),
            StageEvent::Playback { .. } => {
                if let Some(timed) = self.stage.program.as_ref().and_then(|s| s.timed()) {
                    self.media.sync_playback(timed, engine);
                }
            }
            StageEvent::Seek { position } => self.media.seek(*position, engine),
            StageEvent::Frame { .. } | StageEvent::Blackout { .. } | StageEvent::Media { .. } => {}
        }
        Task::none()
    }

    fn program_changed(&mut self) -> Task<Message> {
        let load = self
            .media
            .program_changed(&self.stage, &self.engine.0)
            .map(Message::Loaded);
        Task::batch([load, self.preload()])
    }

    /// Preloads what the operator is most likely to take next: the preview
    /// and the plan's next step. Every YouTube video in the plan starts
    /// downloading, since a download takes about as long as the video.
    fn preload(&mut self) -> Task<Message> {
        for step in self.engine.0.plan().steps() {
            if let Action::Show { resource } = &step.action
                && let Some(video) = YoutubeVideo::of(resource)
            {
                self.downloads.0.fetch(&video);
            }
        }
        let preview = self.stage.preview.as_ref().map(|s| (*s.resource).clone());
        let next = self.engine.0.up_next().and_then(|step| match step.action {
            Action::Show { resource } => Some(resource),
            _ => None,
        });
        self.media
            .preload(preview.into_iter().chain(next))
            .map(Message::Loaded)
    }

    fn view(&self, window: window::Id) -> Element<'_, Message> {
        if window == self.stage_window {
            self.stage_view()
        } else {
            self.dashboard_view()
        }
    }

    /// Black, with the program on top unless it failed. Blackout covers
    /// the program instead of removing it, so a video keeps playing and
    /// still reports its end.
    fn stage_view(&self) -> Element<'_, Message> {
        let Some(scene) = &self.stage.program else {
            return black(Space::new());
        };
        let content: Element<'_, Message> = match (&scene.media, scene.text()) {
            (Media::Failed { .. }, _) => Space::new().into(),
            (_, Some(body)) => text(body)
                .size(64)
                .color(Color::WHITE)
                .align_x(iced::alignment::Horizontal::Center)
                .into(),
            (_, None) => match self.media.content(scene.id) {
                None => Space::new().into(),
                Some(Content::Image(handle)) => fit(image(handle.clone())),
                Some(Content::Pages(pages)) => {
                    let page = &pages[scene.frame().min(pages.len() - 1)];
                    fit(image(page.clone()))
                }
                Some(Content::Video { player, .. }) => VideoPlayer::new(player)
                    .width(Fill)
                    .height(Fill)
                    .content_fit(ContentFit::Contain)
                    .on_end_of_stream(Message::Ended(scene.id))
                    .into(),
            },
        };
        if self.stage.blackout {
            stack![black(content), black(Space::new())].into()
        } else {
            black(content)
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            Subscription::run_with(self.engine.clone(), stage_feed),
            window::close_events().map(Message::Closed),
            iced::event::listen_with(keys::key_event),
        ];
        if self.colors.is_some() {
            subscriptions
                .push(iced::time::every(Duration::from_secs(2)).map(|_| Message::CheckPalette));
        }
        let playing = self
            .stage
            .program
            .as_ref()
            .and_then(|scene| scene.timed())
            .is_some_and(|timed| timed.playback == Playback::Playing);
        if playing {
            subscriptions
                .push(iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick));
        }
        // Section clocks tick while the service runs.
        if self.engine.0.current_step().is_some() {
            subscriptions
                .push(iced::time::every(Duration::from_secs(1)).map(|_| Message::Progress));
        }
        if self.downloads.0.busy() || self.library.0.indexing() {
            subscriptions
                .push(iced::time::every(Duration::from_millis(250)).map(|_| Message::Progress));
        }
        if let Some(script) = &self.script {
            subscriptions.push(Subscription::run_with(
                (
                    self.engine.clone(),
                    self.library.clone(),
                    self.downloads.clone(),
                    script.clone(),
                ),
                run_script,
            ));
        }
        Subscription::batch(subscriptions)
    }
}

/// Sets a section's budget or a step's length.
fn set_length(plan: &mut Plan, id: StepId, value: Option<Duration>) {
    let Some(step) = plan.get_mut(id) else { return };
    match &mut step.action {
        Action::Section { budget } => *budget = value,
        _ => step.estimate = value,
    }
}

fn black<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .center(Fill)
        .style(|_| container::Style::default().background(Color::BLACK))
        .into()
}

fn fit(image: image::Image<image::Handle>) -> Element<'static, Message> {
    image
        .width(Fill)
        .height(Fill)
        .content_fit(ContentFit::Contain)
        .into()
}

fn platform(application_id: &str) -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific {
        application_id: application_id.into(),
        ..window::settings::PlatformSpecific::default()
    }
}

/// The engine's stage feed as messages: its snapshot, then every event. A
/// thread blocks on the std receiver and forwards into the async stream.
fn stage_feed(engine: &Shared<Engine>) -> impl Stream<Item = Message> + use<> {
    let engine = engine.0.clone();
    iced::stream::channel(64, async move |mut out| {
        let StageFeed { state, events } = engine.subscribe();
        let (forward, mut received) = iced::futures::channel::mpsc::unbounded();
        std::thread::spawn(move || {
            for event in events {
                if forward.unbounded_send(event).is_err() {
                    break;
                }
            }
        });
        if out.send(Message::Feed(state)).await.is_err() {
            return;
        }
        while let Some(event) = received.next().await {
            if out.send(Message::Stage(event)).await.is_err() {
                break;
            }
        }
    })
}

/// Runs the script on its own thread, since its commands block (`wait`,
/// `shot`). Reports each command and the end.
fn run_script(
    (engine, library, downloads, script): &(
        Shared<Engine>,
        Shared<Library>,
        Shared<Downloads>,
        Script,
    ),
) -> impl Stream<Item = Message> + use<> {
    let engine = engine.0.clone();
    let library = library.0.clone();
    let downloads = downloads.0.clone();
    let script = script.clone();
    iced::stream::channel(64, async move |mut out| {
        let (acted, mut received) = iced::futures::channel::mpsc::unbounded();
        std::thread::spawn(move || {
            let ui = acted.clone();
            script.run(
                &engine,
                &library,
                &downloads,
                |command| {
                    let _ = ui.unbounded_send(Message::Script(command));
                },
                || {
                    let _ = acted.unbounded_send(Message::Acted);
                },
            );
            let _ = acted.unbounded_send(Message::ScriptDone);
        });
        while let Some(message) = received.next().await {
            if out.send(message).await.is_err() {
                break;
            }
        }
    })
}

/// Resolves after `delay`, timed on its own thread.
fn after(delay: Duration) -> impl Future<Output = ()> {
    let (done, wait) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let _ = done.send(());
    });
    async move {
        let _ = wait.await;
    }
}

/// A live search's events as messages, tagged with its ticket. The stream
/// blocks, so a thread drains it.
fn live_search(stream: SearchStream) -> Task<Message> {
    let ticket = stream.ticket();
    let (send, events) = iced::futures::channel::mpsc::unbounded();
    std::thread::spawn(move || {
        for event in stream {
            if send
                .unbounded_send(Message::Searched(ticket, Arc::new(event)))
                .is_err()
            {
                return;
            }
        }
        let _ = send.unbounded_send(Message::SearchDone(ticket));
    });
    Task::run(events, |message| message)
}
