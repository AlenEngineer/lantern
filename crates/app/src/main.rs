//! The lantern desktop app: an operator dashboard and a fullscreen stage
//! window for the projector, both following one [`Engine`].
//!
//! It reads its media folders and Bibles from a config file (see
//! [`config::load`]) and indexes them in the background.
//!
//! Set `LANTERN_SCRIPT=<file>` to drive the engine from a script and exit
//! after its last line (see [`script`]). `LANTERN_SHOT=<program>` is run
//! for each `shot NAME` in it.

mod bundle;
mod config;
mod cut;
mod dashboard;
mod drag;
mod fit;
mod keys;
mod length;
mod library;
mod look;
mod outputs;
mod palette;
mod paths;
mod plan_edit;
mod plan_pane;
mod plans;
mod ready;
mod script;
mod search;
mod settings;
mod stage;
mod thumbs;
mod watch;
mod youtube;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use iced::futures::{SinkExt, Stream, StreamExt};
use iced::widget::{Space, container, image, operation, scrollable, stack};
use iced::{Color, ContentFit, Element, Fill, Subscription, Task, window};
use iced_video_player::VideoPlayer;
use lantern_engine::{
    Action, Engine, Look, Media, MediaReport, Plan, Playback, Query, ResourceId, SceneId,
    SearchEvent, SearchStream, StageEvent, StageFeed, StageState, Step, StepId,
};

use crate::config::Config;
use crate::dashboard::{NAME_BOX, RESULTS, SEARCH_BOX};
use crate::drag::{Drag, Half};
use crate::keys::Op;
use crate::library::Library;
use crate::palette::Palette;
use crate::plans::{Plans, Store, Switch};
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
    if let Err(err) = keys::configure(&config.keys) {
        log(format!("keys: {err} Using the usual keys."));
    }
    let config_status = Arc::new(config_status);
    let engine = Shared(Arc::new(Engine::new()));
    let library = Shared(Library::start(engine.0.clone(), config.clone()));
    let videos = stage::cache_dir().join("youtube");
    // Before anything loads, so no player opens a file being deleted.
    for gone in youtube::forget_unused(&videos, youtube::KEEP, SystemTime::now()) {
        log(format!(
            "youtube: deleted {}, unused for 30 days",
            gone.display()
        ));
    }
    let downloads = Shared(Downloads::new(videos));
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
    // The window itself is clear; the dashboard paints its own ground,
    // as solid as the settings say, and the stage paints black.
    .style(|app: &App, _| iced::theme::Style {
        background_color: Color::TRANSPARENT,
        text_color: app.palette.on_surface,
    })
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

/// Two clicks on the same thing this close together are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// How many plan edits Ctrl+Z can take back.
const UNDO_DEPTH: usize = 50;

/// What a click lands on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Clicked {
    Result(ResourceId),
    Step(StepId),
    /// Something on the plan's standby shelf.
    Standby(ResourceId),
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
    /// A window's size, at the start and on every resize.
    Resized(window::Id, iced::Size),
    /// The search box changed.
    Query(String),
    /// The pause after this edit of the query is over.
    Paused(u64),
    /// An event from the live search with this ticket.
    Searched(u64, Arc<SearchEvent>),
    /// Every source answered the live search with this ticket.
    SearchDone(u64),
    /// A result or plan step was clicked: a single click lines it up, a
    /// double click also shows it.
    Click(Clicked),
    /// The pointer went over half of a plan row, or off it.
    Hover(Option<(StepId, Half)>),
    /// The pointer left this plan row. Only clears the hover if it's still
    /// this row's: entering the next row may come first.
    Unhover(StepId),
    /// The pointer left the plan list: a drag let go now goes nowhere.
    LeftPlan,
    /// The left button went down (`true`) or up in a window; `None` for a
    /// script, which has no window.
    Pointer(Option<window::Id>, bool),
    /// The pointer moved in a window, while a row is held.
    Cursor(Option<window::Id>, iced::Point),
    /// A result was picked, which previews it.
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
    /// A plan step was picked: it becomes the selection, where new steps
    /// go, and is lined up in the preview.
    Select(StepId),
    Remove(StepId),
    /// Takes this off the plan's standby shelf.
    Unstandby(ResourceId),
    /// Adds a section, live slot, black screen or note after the selection.
    AddStep(plan_edit::NewStep),
    /// "Set a time" on a section: select it and focus its time field.
    /// Edits a part of a step in its row, or with `None` is done editing.
    EditStep(Option<(StepId, plan_edit::Part)>),
    /// What a live slot searches when the service reaches it ("!yt", "!b").
    SlotSearch(StepId, String),
    /// A step's or section's new name.
    Rename(StepId, String),
    /// Turn a step's "go on when done" on or off.
    SetAdvance(StepId, bool),
    /// Open the settings on this plan: its name, look, template, copy, delete.
    PlanSettings,
    /// The plan's name, as typed in its settings.
    RenamePlan(String),
    /// Makes the open plan a template, or a plan again.
    PlanTemplate(bool),
    /// When the open plan's service starts, as typed in its settings.
    PlanStarts(String),
    /// Saves the open plan as a copy with its own file, and opens the copy.
    DuplicatePlan,
    /// Writes the open plan and its media to one file to take along.
    ExportPlan,
    /// The export finished: where it went, or why it didn't.
    Exported(Result<String, String>),
    /// Opens or closes "Import a plan" in the list of plans.
    ToggleImport,
    /// The path typed for the plan to import.
    ImportPath(String),
    /// Imports the exported plan in this file; `None` for the one typed.
    Import(Option<PathBuf>),
    /// The import finished: the new plan's file, or why there's none.
    Imported(Result<PathBuf, String>),
    /// Deletes the open plan; the first press asks, the second does.
    DeletePlan,
    /// What was typed in a step's length or a section's budget.
    Length(StepId, String),
    /// Moves a step up (-1) or down (1).
    Move(StepId, isize),
    /// A change in the settings view.
    Settings(settings::Edit),
    /// Time to see whether the stage should move to its screen.
    CheckOutputs,
    /// Time to see whether a watched folder changed the index.
    CheckIndex,
    /// Show slide (verse, stanza, page) N of what's on stage.
    GotoFrame(usize),
    /// Back to the plan: shows its current step again.
    Resume,
    /// Writes the settings to the config file and applies them.
    SaveSettings,
    /// A saved plan in the open list was clicked.
    /// Start a new plan from the template saved here.
    FromTemplate(std::path::PathBuf),
    /// Make the saved plan here a template, or a plan again.
    SetTemplate(std::path::PathBuf, bool),
    OpenPlan(PathBuf),
    /// The operator agreed to lose the unsaved plan.
    DiscardPlan,
    /// Yes, replace the plan that's running.
    LeavePlan,
    /// The line between two panes was pressed: drag sizes the side pane,
    /// a double click lets it size itself.
    ResizePane(dashboard::Side),
    /// The key pressed while the settings waited for one, as the config
    /// writes it; `None` for Escape, which gives up.
    KeyRecorded(Option<String>),
    /// Opens or closes the plan's "Add step" row.
    ToggleAdd,
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
    /// How slide text is split to fit the stage.
    slides: fit::Splitter,
    /// The stage's look now: the open plan's own, or else the default.
    look: Look,
    /// Whether the desktop was last told to blur behind the dashboard;
    /// `None` until its window is open, or once telling it failed.
    blur_told: Option<bool>,
    /// The dashboard's window is open, so the desktop can be told.
    dashboard_open: bool,
    /// Another plan was opened with something on stage: what's there keeps
    /// the look it went up with, until something else goes on.
    look_held: bool,
    /// What the stage's screen was last seen as, to move it only on change.
    output_watch: outputs::Watch,
    /// The library's change count the search last followed.
    index_changes: u64,
    /// Thumbnails for results and plan steps.
    thumbs: thumbs::Thumbs,
    /// The roles file the palette follows, if the config names one.
    colors: Option<palette::Source>,
    /// Why the roles file couldn't be read, while it can't.
    palette_error: Option<String>,
    script: Option<Script>,
    stage_window: window::Id,
    /// The dashboard window's size, to lay out for the room there is.
    dashboard_size: iced::Size,
    /// The stage window's mirror, built only from the engine's feed.
    stage: StageState,
    media: Stage,
    search: Search,
    /// The selected plan step: where new steps go, and what Enter puts on
    /// stage while the search box is empty.
    selected: Option<StepId>,
    /// The last click on a result or step, to tell a double click.
    last_click: Option<(Clicked, Instant)>,
    /// The plan row (and its half) under the pointer.
    hovered: Option<(StepId, Half)>,
    /// The plan row the button went down on, while it's held.
    drag: Option<Drag>,
    /// Something just went on stage and nothing's been typed since: the
    /// arrows step its slides even with text in the search box.
    stepping: bool,
    /// A search result just went on stage: what's typed next starts a new
    /// search instead of adding to the old one.
    spent: bool,
    /// The plan's "Add step" row is open, showing the kinds of step.
    /// and which of them the arrows are on, for Enter.
    adding: Option<usize>,
    /// The search is open in the middle pane. Closed, the middle shows
    /// the keys, and the arrows and Enter are the plan pane's.
    search_open: bool,
    /// What the open plan asked of the library when it was last given it
    /// (its own folders, its Bibles), to notice when another plan or an
    /// edit asks for something else.
    plan_needs: library::PlanNeeds,
    /// The side panes' widths as dragged; `None` sizes one for the window.
    pub(crate) plan_dragged: Option<f32>,
    pub(crate) stage_dragged: Option<f32>,
    /// The side pane whose edge is held, and when it was last pressed, to
    /// tell a double click.
    pub(crate) resizing: Option<dashboard::Side>,
    seam_pressed: Option<(dashboard::Side, Instant)>,
    /// An export or import is copying files.
    pub(crate) copying: bool,
    /// "Import a plan" is open in the list of plans: the path typed, and
    /// the exported plans found lying about.
    pub(crate) importing: Option<(String, Vec<PathBuf>)>,
    /// The plan's start time while it's being typed and doesn't read as
    /// one yet ("10:").
    pub(crate) starts_typed: Option<String>,
    /// Delete was pressed once in the plan's settings: again deletes.
    pub(crate) deleting_plan: bool,
    /// The part of a step being edited in its row.
    editing: Option<(StepId, plan_edit::Part)>,
    /// The saved plan the arrows are on in the list of plans, for Enter.
    list_pick: Option<PathBuf>,
    /// The plan before each edit, newest last, for Ctrl+Z.
    undo: Vec<Plan>,
    /// The step whose fields are being typed in, so its keystrokes undo as
    /// one edit.
    typing: Option<StepId>,
    /// The plan step the service was at, to notice arriving at a new one.
    arrived_at: Option<StepId>,
    /// What's on stage (scene and frame) and since when, to move a still
    /// step that goes on by itself after its length.
    shown_since: Option<(Option<SceneId>, usize, Instant)>,
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
            // Always able to be see-through: how much is the config's,
            // and can change while the window is open.
            transparent: true,
            blur: !config.no_blur,
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
        let layout = store.layout();
        let plans = Plans::start(store, &engine.0);
        let mut app = App {
            media: Stage::new(downloads.0.clone()),
            plans,
            engine,
            library,
            config_status,
            palette: Palette::default(),
            colors: config.colors.clone().map(palette::Source::new),
            slides: fit::Splitter::new(&config.stage),
            look: config.stage.clone(),
            look_held: false,
            blur_told: None,
            dashboard_open: false,
            output_watch: outputs::Watch::default(),
            index_changes: 0,
            thumbs: thumbs::Thumbs::start(),
            config,
            settings: None,
            home: config::env("HOME").map(PathBuf::from),
            palette_error: None,
            downloads,
            script,
            stage_window,
            dashboard_size: iced::Size::new(1280.0, 720.0),
            stage: StageState::default(),
            search: Search::default(),
            selected: None,
            last_click: None,
            hovered: None,
            drag: None,
            stepping: false,
            spent: false,
            adding: None,
            search_open: false,
            plan_needs: library::PlanNeeds::default(),
            plan_dragged: layout.0,
            stage_dragged: layout.1,
            resizing: None,
            seam_pressed: None,
            copying: false,
            importing: None,
            starts_typed: None,
            deleting_plan: false,
            editing: None,
            list_pick: None,
            undo: Vec::new(),
            typing: None,
            arrived_at: None,
            shown_since: None,
            drafts: HashMap::new(),
            scrub: None,
        };
        app.check_palette();
        // The plan reopened at start may bring folders of its own.
        app.sync_library();
        // Dashboard first: on a single monitor, Hyprland drops a fullscreen
        // window back to tiled when another window maps on its workspace.
        // Then the keyboard goes back to the dashboard, into the search box.
        let open = open_dashboard
            .discard()
            .chain(open_stage.discard())
            .chain(window::gain_focus(dashboard_window))
            .chain(operation::focus(SEARCH_BOX))
            .chain(window::size(stage_window).map(move |size| Message::Resized(stage_window, size)))
            .chain(
                window::size(dashboard_window)
                    .map(move |size| Message::Resized(dashboard_window, size)),
            );
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
        let task = self.handle(message);
        self.sync_look();
        self.sync_blur();
        task
    }

    /// Has the desktop blur behind the dashboard, or not, as the settings
    /// say: the switch as it stands while they're open, else as saved.
    /// Only on a change, and only where the desktop can be told (a
    /// window asks for blur itself when it opens, which is enough on
    /// the others that can).
    fn sync_blur(&mut self) {
        if !self.dashboard_open || !outputs::supported() {
            return;
        }
        let wanted = match &self.settings {
            Some(draft) => draft.blur,
            None => !self.config.no_blur,
        };
        // Blurred is how a window starts: nothing to say until it isn't.
        if self.blur_told.unwrap_or(true) == wanted {
            return;
        }
        match outputs::blur_dashboard(wanted) {
            Ok(true) => self.blur_told = Some(wanted),
            Ok(false) => {}
            Err(err) => {
                log(format!("blur: {err}"));
                // Not asked again on every message.
                self.blur_told = Some(wanted);
            }
        }
    }

    /// Follows the look to the open plan's (or the default when it has
    /// none), whatever changed it: opening a plan, editing its look, saving
    /// the settings. Text is split again only when the look changed.
    fn sync_look(&mut self) {
        let look = self
            .engine
            .0
            .plan()
            .look
            .unwrap_or_else(|| self.config.stage.clone());
        if look != self.look && !self.look_held {
            self.look = look;
            if self.slides.set_look(&self.look) {
                self.report_slides();
            }
        }
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Feed(state) => {
                self.stage = state;
                return self.program_changed();
            }
            Message::Stage(event) => {
                let task = self.stage_event(&event);
                // A new scene or slide starts its time the moment it's up.
                self.mark_shown();
                return task;
            }
            Message::Loaded(loaded) => {
                self.media.loaded(&loaded, &self.stage, &self.engine.0);
                // The next of the plan's pictures to keep loaded.
                return self.media.keep_more().map(Message::Loaded);
            }
            Message::Ended(scene) => {
                self.engine.0.report(scene, MediaReport::Ended);
                // A step that goes on by itself has moved the plan.
                return Task::batch([self.preload(), self.arrive()]);
            }
            Message::Tick => {
                if let Some((scene, video)) = self.media.video() {
                    let position = video.position();
                    self.engine.0.report(scene, MediaReport::Position(position));
                }
            }
            Message::Progress => return self.go_on(),
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
                    Ui::Click(n) => match self.search.pick_nth(n) {
                        Some(resource) => {
                            self.update(Message::Click(Clicked::Result(resource.id.clone())))
                        }
                        None => {
                            log(format!("dashboard: no result {}", n + 1));
                            Task::none()
                        }
                    },
                    Ui::Drag(from, to, half) => {
                        let steps = self.engine.0.plan().steps().to_vec();
                        match (steps.get(from), steps.get(to)) {
                            (Some(from), Some(to)) => {
                                // Press on the row, move past the slop, then
                                // over the target's half, as a hand would.
                                let _ = self.update(Message::Hover(Some((from.id, Half::Upper))));
                                let _ = self.update(Message::Pointer(None, true));
                                let _ = self
                                    .update(Message::Cursor(None, iced::Point::new(240.0, 200.0)));
                                let _ = self
                                    .update(Message::Cursor(None, iced::Point::new(260.0, 240.0)));
                                self.update(Message::Hover(Some((to.id, half))))
                            }
                            _ => {
                                log(format!(
                                    "dashboard: no plan step {} or {}",
                                    from + 1,
                                    to + 1
                                ));
                                Task::none()
                            }
                        }
                    }
                    Ui::Drop => self.update(Message::Pointer(None, false)),
                    Ui::Slide(n) => self.update(Message::GotoFrame(n)),
                    Ui::Output(name) => {
                        let output = (name != "automatic").then_some(name);
                        self.update(Message::Settings(settings::Edit::Output(output)))
                    }
                    Ui::Length(n, text) => match self.engine.0.plan().steps().get(n) {
                        Some(step) => self.update(Message::Length(step.id, text)),
                        None => Task::none(),
                    },
                    Ui::GoOn(n) => match self.engine.0.plan().steps().get(n) {
                        Some(step) => self.update(Message::SetAdvance(step.id, true)),
                        None => Task::none(),
                    },
                    Ui::PlanLook(colour) => Task::batch([
                        self.update(Message::PlanSettings),
                        self.update(Message::Settings(settings::Edit::OwnLook)),
                        self.update(Message::Settings(settings::Edit::Background(colour))),
                    ]),
                    Ui::Step(n) => match self.engine.0.plan().steps().get(n) {
                        Some(step) => self.update(Message::Click(Clicked::Step(step.id))),
                        None => {
                            log(format!("dashboard: no plan step {}", n + 1));
                            Task::none()
                        }
                    },
                    Ui::Path(text) => {
                        let library = settings::Edit::Category(settings::Category::Library);
                        let _ = self.update(Message::Settings(library));
                        if self.settings.as_ref().is_some_and(|d| d.folder_rows() == 0) {
                            let _ = self.update(Message::Settings(settings::Edit::AddFolder));
                        }
                        self.update(Message::Settings(settings::Edit::FolderPath(0, text)))
                    }
                    Ui::Font(text) => Task::batch([
                        self.update(Message::Settings(settings::Edit::Font(text))),
                        operation::snap_to_end(settings::SCROLL),
                    ]),
                    Ui::Press(op) => self.op(op),
                    Ui::Template(name) | Ui::FromTemplate(name)
                        if self.plans.find(&name).is_none() =>
                    {
                        log(format!("dashboard: no saved plan named {name}"));
                        Task::none()
                    }
                    Ui::Template(name) => {
                        let path = self.plans.find(&name).expect("checked above");
                        self.update(Message::SetTemplate(path, true))
                    }
                    Ui::FromTemplate(name) => {
                        let path = self.plans.find(&name).expect("checked above");
                        self.update(Message::FromTemplate(path))
                    }
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
            Message::Hover(row) => {
                self.hovered = row;
                self.aim();
            }
            Message::Unhover(row) => {
                if self.hovered.is_some_and(|(over, _)| over == row) {
                    self.hovered = None;
                }
            }
            Message::LeftPlan => {
                if let Some(drag) = &mut self.drag {
                    drag.landing = None;
                }
            }
            Message::Pointer(window, _) | Message::Cursor(window, _)
                if window == Some(self.stage_window) => {}
            Message::ResizePane(side) => {
                let now = Instant::now();
                let double = self.seam_pressed.take().is_some_and(|(last, at)| {
                    last == side && now.duration_since(at) < DOUBLE_CLICK
                });
                if double {
                    // Back to sizing itself for the window.
                    match side {
                        dashboard::Side::Plan => self.plan_dragged = None,
                        dashboard::Side::Stage => self.stage_dragged = None,
                    }
                    self.keep_layout();
                } else {
                    self.seam_pressed = Some((side, now));
                    self.resizing = Some(side);
                }
            }
            Message::Cursor(_, at) if self.resizing.is_some() => {
                let side = self.resizing.expect("checked above");
                let width = Some(self.dragged_to(side, at.x));
                match side {
                    dashboard::Side::Plan => self.plan_dragged = width,
                    dashboard::Side::Stage => self.stage_dragged = width,
                }
            }
            Message::Pointer(_, false) if self.resizing.is_some() => {
                self.resizing = None;
                self.drag = None;
                self.keep_layout();
            }
            Message::Cursor(_, at) => {
                if let Some(drag) = &mut self.drag {
                    drag.to(at);
                }
                self.aim();
            }
            Message::Pointer(_, true) => {
                let plan = self.engine.0.plan();
                self.drag = self.hovered.map(|(step, _)| Drag::new(&plan, step));
            }
            Message::Pointer(_, false) => {
                let dropped = self.drag.take().filter(|drag| drag.moving);
                if let Some((drag, at)) = dropped.and_then(|d| d.landing.map(|at| (d, at))) {
                    self.remember(None);
                    self.engine
                        .0
                        .edit_plan(|plan| plan.move_steps(&drag.carried, at));
                    return self.preload();
                }
            }
            Message::Click(target) => {
                let now = Instant::now();
                let double = self.last_click.take().is_some_and(|(last, at)| {
                    last == target && now.duration_since(at) < DOUBLE_CLICK
                });
                if !double {
                    self.last_click = Some((target.clone(), now));
                }
                let lined_up = match target {
                    Clicked::Result(id) => self.update(Message::Pick(id)),
                    Clicked::Step(step) => self.update(Message::Select(step)),
                    Clicked::Standby(id) => self.line_up_standby(&id),
                };
                if double {
                    return Task::batch([lined_up, self.op(Op::Take)]);
                }
                return lined_up;
            }
            Message::Pick(id) => {
                if let Some(resource) = self.search.pick(&id) {
                    self.engine.0.preview(resource);
                }
            }
            Message::Bang(bang) => return self.complete(format!("!{bang} ")),
            Message::Key { op, only_if_empty } => {
                if !only_if_empty || self.search.query.is_empty() || self.stepping {
                    return self.op(op);
                }
            }
            // Typing goes to the search box, which settings cover.
            Message::Type(_) if self.settings.is_some() => {}
            // With the search closed there's nowhere to type.
            Message::Type(_) if !self.search_open => {}
            Message::Type(typed) => {
                self.stepping = false;
                let query = if std::mem::take(&mut self.spent) {
                    typed
                } else {
                    format!("{}{typed}", self.search.query)
                };
                return self.complete(query);
            }
            Message::CheckPalette => self.check_palette(),
            Message::Settings(edit) => {
                // Picking a screen moves the stage there at once.
                if let settings::Edit::Output(Some(output)) = &edit {
                    move_stage(output);
                }
                // A folder row just added is the one to type in.
                let added = matches!(edit, settings::Edit::AddPlanFolder);
                let chosen = self.settings.as_mut().and_then(|draft| draft.edit(edit));
                if added
                    && let Some(field) = self.settings.as_ref().and_then(|d| d.last_plan_folder())
                {
                    return operation::focus(field.id());
                }
                // A key waiting to be pressed takes the next key press.
                keys::record(
                    self.settings
                        .as_ref()
                        .is_some_and(|d| d.recording.is_some()),
                );
                // A chosen path is longer than what was typed: cursor to its end.
                if let Some(field) = chosen {
                    return operation::move_cursor_to_end(field.id());
                }
            }
            Message::KeyRecorded(pressed) => {
                let Some(draft) = &mut self.settings else {
                    keys::record(false);
                    return Task::none();
                };
                match (pressed, draft.recording) {
                    (Some(written), Some(at)) => match written.parse::<keys::Bind>() {
                        Ok(_) => {
                            draft.edit(settings::Edit::Key(at, written));
                            keys::record(false);
                        }
                        // Keeps waiting: a letter alone would type.
                        Err(why) => draft.notice = Some(Err(format!("{why}."))),
                    },
                    _ => {
                        draft.edit(settings::Edit::StopRecording);
                        keys::record(false);
                    }
                }
            }
            Message::SaveSettings => self.save_settings(),
            Message::CheckOutputs => self.check_outputs(),
            Message::Resume => {
                self.engine.0.resume();
                self.engine.0.clear_preview();
                self.selected = None;
                return Task::batch([self.preload(), self.arrive()]);
            }
            Message::GotoFrame(frame) => {
                self.engine.0.goto_frame(frame);
                self.stepping = true;
            }
            Message::CheckIndex => {
                let changes = self.library.0.changes();
                if changes != self.index_changes {
                    self.index_changes = changes;
                    self.search.refresh_index(&self.engine.0);
                }
            }
            Message::Select(step) => {
                if self.editing.is_some_and(|(id, _)| id != step) {
                    self.editing = None;
                }
                self.selected = Some(step);
                if let Some(Action::Show { resource }) = self.cued().map(|s| s.action) {
                    self.search.unpick();
                    self.engine.0.preview(Arc::new(resource));
                } else {
                    self.engine.0.clear_preview();
                }
                return self.preload();
            }
            Message::Unstandby(id) => {
                self.remember(None);
                self.engine
                    .0
                    .edit_plan(|plan| plan.standby.retain(|resource| resource.id != id));
                // It was the one lined up: nothing is now.
                let preview = self.engine.0.stage().preview;
                if self.selected.is_none() && preview.is_some_and(|scene| scene.resource.id == id) {
                    self.engine.0.clear_preview();
                }
                return self.preload();
            }
            Message::Remove(step) => {
                if self.selected == Some(step) {
                    self.selected = None;
                }
                self.editing = None;
                self.remember(None);
                self.engine.0.edit_plan(|plan| plan.remove(step));
                return self.preload();
            }
            Message::AddStep(new) => {
                let (label, action) = new.step();
                self.adding = None;
                self.remember(None);
                let id = self.engine.0.add(self.selected, label, action);
                // A new step opens on its name.
                return self.update(Message::EditStep(Some((id, plan_edit::Part::Name))));
            }
            Message::EditStep(None) => self.editing = None,
            Message::EditStep(Some((id, part))) => {
                if self.selected != Some(id) {
                    self.selected = Some(id);
                    self.engine.0.clear_preview();
                }
                self.editing = Some((id, part));
                let field = plan_edit::part_id(id, part);
                return Task::batch([
                    operation::focus(field.clone()),
                    operation::select_all(field),
                ]);
            }
            Message::SlotSearch(id, text) => {
                self.remember(Some(id));
                self.engine.0.edit_plan(|plan| {
                    if let Some(Step {
                        action: Action::Live { hint, .. },
                        ..
                    }) = plan.get_mut(id)
                    {
                        *hint = Some(text).filter(|t| !t.trim().is_empty()).map(Query::new);
                    }
                })
            }
            Message::PlanSettings => {
                self.starts_typed = None;
                let mut draft = self
                    .settings
                    .take()
                    .unwrap_or_else(|| self.settings_draft());
                draft.edit(settings::Edit::Category(settings::Category::Plan));
                self.settings = Some(draft);
                return operation::snap_to(settings::SCROLL, scrollable::RelativeOffset::START);
            }
            Message::RenamePlan(name) => {
                self.engine.0.edit_plan(|plan| plan.title = name);
            }
            Message::PlanTemplate(template) => {
                self.engine.0.edit_plan(|plan| plan.template = template);
            }
            Message::PlanStarts(text) => {
                // Only what a time is made of; kept as typed until it
                // reads as one. Emptied, the plan has no start time.
                if length::typeable(&text) {
                    let starts = lantern_engine::TimeOfDay::parse(&text);
                    if starts.is_some() || text.trim().is_empty() {
                        self.engine.0.edit_plan(|plan| plan.starts = starts);
                    }
                    self.starts_typed = Some(text);
                }
            }
            Message::DuplicatePlan => {
                self.deleting_plan = false;
                if self.plans.duplicate(&self.engine.0) {
                    self.settings = None;
                    return operation::focus(SEARCH_BOX);
                }
            }
            Message::ExportPlan if self.copying => {}
            Message::ExportPlan => {
                let plan = self.engine.0.plan();
                let home = self.home.clone();
                // Beside the user's documents, where it's easy to find.
                let into = match &home {
                    Some(home) if home.join("Documents").is_dir() => home.join("Documents"),
                    Some(home) => home.clone(),
                    None => PathBuf::from("."),
                };
                let file = self.config_status.path().to_path_buf();
                let videos: Vec<PathBuf> = plan
                    .steps()
                    .iter()
                    .filter_map(|step| match &step.action {
                        Action::Show { resource } => Some(resource),
                        _ => None,
                    })
                    .chain(&plan.standby)
                    .filter_map(YoutubeVideo::of)
                    .filter_map(|video| match self.downloads.0.state(&video.id) {
                        Some(youtube::Download::Ready(path)) => Some(path),
                        _ => None,
                    })
                    .collect();
                self.copying = true;
                self.plans.notice = Some(plans::Notice::Done("Exporting…".into()));
                return Task::perform(
                    in_thread(move || {
                        let dir = file.parent().unwrap_or(std::path::Path::new("."));
                        let resolve =
                            |path: &std::path::Path| config::resolve(path, dir, home.as_deref());
                        bundle::export(&plan, &into, &resolve, &videos)
                            .map(|exported| exported.said(home.as_deref()))
                    }),
                    Message::Exported,
                );
            }
            Message::Exported(result) => {
                self.copying = false;
                self.plans.notice = Some(match result {
                    Ok(said) => plans::Notice::Done(said),
                    Err(why) => plans::Notice::Failed(why),
                });
            }
            Message::ToggleImport => {
                self.importing = match self.importing {
                    Some(_) => None,
                    None => {
                        let user = config::env("USER");
                        let places = bundle::places(self.home.as_deref(), user.as_deref());
                        Some((String::new(), bundle::find(&places)))
                    }
                };
                if self.importing.is_some() {
                    return operation::focus(dashboard::IMPORT_BOX);
                }
            }
            Message::ImportPath(text) => {
                if let Some((path, _)) = &mut self.importing {
                    *path = text;
                }
            }
            Message::Import(_) if self.copying => {}
            Message::Import(picked) => {
                let typed = self
                    .importing
                    .as_ref()
                    .map(|(path, _)| path.trim().to_string());
                let from = match picked {
                    Some(path) => path,
                    None => match typed.filter(|path| !path.is_empty()) {
                        Some(path) => config::resolve(
                            std::path::Path::new(&path),
                            std::path::Path::new("."),
                            self.home.as_deref(),
                        ),
                        None => return Task::none(),
                    },
                };
                let store = self.plans.store().clone();
                let videos = stage::cache_dir().join("youtube");
                self.copying = true;
                self.plans.notice = Some(plans::Notice::Done("Importing…".into()));
                return Task::perform(
                    in_thread(move || {
                        let plan = bundle::import(&from, store.bundles(), &videos)?;
                        store
                            .adopt(plan.clone())
                            .map_err(|err| format!("Couldn't save “{}”: {err}", plan.title))
                    }),
                    Message::Imported,
                );
            }
            Message::Imported(result) => {
                self.copying = false;
                match result {
                    Ok(path) => {
                        self.importing = None;
                        self.plans.notice = None;
                        let opening = self.switch_plan(Switch::Open(path.clone()));
                        // Says it's the imported one that's open, by the
                        // name it got here; or, while a question waits,
                        // that it's in the list.
                        let title = self.engine.0.plan().title;
                        self.plans.notice = Some(plans::Notice::Done(
                            if self.plans.file() == Some(path.as_path()) {
                                format!("Imported “{title}”")
                            } else {
                                "Imported: it's in your plans".into()
                            },
                        ));
                        return opening;
                    }
                    Err(why) => self.plans.notice = Some(plans::Notice::Failed(why)),
                }
            }
            Message::DeletePlan if !self.deleting_plan => self.deleting_plan = true,
            Message::DeletePlan => {
                self.deleting_plan = false;
                if self.plans.delete(&self.engine.0) {
                    self.settings = None;
                    self.selected = None;
                    self.engine.0.clear_preview();
                    return self.preload_if(true);
                }
            }
            Message::Rename(id, label) => {
                self.remember(Some(id));
                self.engine.0.edit_plan(|plan| {
                    if let Some(step) = plan.get_mut(id) {
                        step.label = label;
                    }
                })
            }
            Message::SetAdvance(id, on) => {
                self.remember(None);
                self.engine.0.edit_plan(|plan| {
                    if let Some(step) = plan.get_mut(id) {
                        step.advance = on;
                    }
                })
            }
            // A length field takes only what a length is made of.
            Message::Length(_, typed) if !length::typeable(typed.trim()) => {}
            Message::Length(id, typed) => {
                if let Some(value) = length::parse(&typed) {
                    self.remember(Some(id));
                    self.engine.0.edit_plan(|plan| set_length(plan, id, value));
                }
                self.drafts.insert(id, typed);
            }
            Message::Move(id, by) => {
                self.remember(None);
                self.engine.0.edit_plan(|plan| {
                    if let Some(at) = plan.position(id) {
                        plan.move_to(id, at.saturating_add_signed(by));
                    }
                });
                return self.preload();
            }
            Message::FromTemplate(path) => return self.switch_plan(Switch::FromTemplate(path)),
            Message::SetTemplate(path, template) => {
                if let Err(err) = self.plans.store().set_template(&path, template) {
                    log(format!("plans: {}: {err}", path.display()));
                }
                // The list shows it in its new group.
                self.plans.refresh();
            }
            Message::OpenPlan(path) => return self.switch_plan(Switch::Open(path)),
            Message::LeavePlan => {
                let replaced = self.plans.leave(&self.engine.0);
                return self.switched(replaced);
            }
            Message::ToggleAdd => {
                self.adding = match self.adding {
                    Some(_) => None,
                    None => Some(0),
                }
            }
            Message::DiscardPlan => {
                let replaced = self.plans.discard(&self.engine.0);
                return self.switched(replaced);
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
            Message::Resized(window, size) if window != self.stage_window => {
                self.dashboard_open = true;
                self.dashboard_size = size;
            }
            Message::Resized(_, size) => {
                if self.slides.set_stage(size) {
                    self.report_slides();
                }
            }
        }
        Task::none()
    }

    /// Index results now, the live sources once typing pauses.
    fn query(&mut self, query: String) -> Task<Message> {
        // A search from anywhere (a bang row, a live slot, a script) opens it.
        self.search_open = true;
        self.stepping = false;
        // What was searched is on stage: typing starts the next search.
        let fresh = std::mem::take(&mut self.spent)
            .then(|| typed_into(&self.search.query, &query))
            .flatten();
        let restarted = fresh.is_some();
        let had_bang = self.search.bang().is_some();
        let edit = self.search.edit(fresh.unwrap_or(query), &self.engine.0);
        let searched = self.searched(edit);
        if restarted {
            return Task::batch([searched, operation::move_cursor_to_end(SEARCH_BOX)]);
        }
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
            // Without a search, Enter takes what the arrows are on in the
            // plan pane: a kind of step, or a saved plan.
            Op::Enter if !self.searching() && self.adding.is_some() => {
                let (kind, _) = plan_edit::NewStep::ALL
                    [self.adding.unwrap_or(0) % plan_edit::NewStep::ALL.len()];
                return self.update(Message::AddStep(kind));
            }
            Op::Enter
                if !self.searching()
                    && self.list_pick.is_some()
                    && matches!(self.plans.mode, plans::Mode::Browsing(_)) =>
            {
                let Some(path) = self.list_pick.take() else {
                    return Task::none();
                };
                let template = match &self.plans.mode {
                    plans::Mode::Browsing(entries) => entries
                        .iter()
                        .find(|entry| entry.path == path)
                        .is_some_and(|entry| entry.summary.as_ref().is_ok_and(|s| s.template)),
                    _ => false,
                };
                return self.switch_plan(if template {
                    Switch::FromTemplate(path)
                } else {
                    Switch::Open(path)
                });
            }
            Op::Enter => {
                if self.search.completion(&engine).is_some() {
                    return self.op(Op::Complete);
                }
                if let Some(stream) = self.search.start_on_enter(&engine) {
                    return live_search(stream);
                }
                // With nothing searched, Enter shows the selected plan step;
                // while searching, the result.
                if self.search.text().is_empty() && self.cued().is_some() {
                    return self.op(Op::Take);
                }
                if self.search.picked().is_none() {
                    if let Some(first) = self.search.pick_nth(0) {
                        engine.preview(first);
                        return self.scroll_to_pick();
                    }
                    // Nothing found, or still searching: Enter has nothing
                    // to show, and must not show what was looked at before.
                    if self.search.live != Live::Idle || !self.search.query.trim().is_empty() {
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
                    self.selected = None;
                    self.spent = !self.search.query.is_empty();
                    if let Err(err) = engine.go_to(step.id) {
                        log(format!("dashboard: {err}"));
                    }
                    // The step was only lined up to look at: Next goes
                    // back to the plan's next step.
                    engine.clear_preview();
                    return Task::batch([self.preload(), self.arrive()]);
                }
                if engine.take() {
                    self.search.unpick();
                    self.stepping = true;
                    self.spent = !self.search.query.is_empty();
                }
            }
            Op::AddToPlan => {
                // While searching, the result picked or else the first;
                // otherwise what's in preview.
                let resource = if self.search.text().is_empty() {
                    engine.stage().preview.map(|scene| scene.resource.clone())
                } else {
                    self.search
                        .picked()
                        .cloned()
                        .or_else(|| self.search.first())
                };
                if let Some(resource) = resource {
                    self.remember(None);
                    self.selected = Some(engine.add_resource(self.selected, &resource));
                    return self.preload();
                }
            }
            Op::Standby => {
                // While searching, the result picked or else the first;
                // otherwise what's lined up, such as the selected step's.
                let found = self
                    .search
                    .picked()
                    .cloned()
                    .or_else(|| self.search.first())
                    .filter(|_| self.searching() && !self.search.query.trim().is_empty());
                let resource =
                    found.or_else(|| engine.stage().preview.map(|scene| scene.resource.clone()));
                if let Some(resource) = resource {
                    let resource = (*resource).clone();
                    let kept =
                        engine.edit_plan(|plan| plan.standby.iter().any(|r| r.id == resource.id));
                    if !kept {
                        self.remember(None);
                        engine.edit_plan(|plan| plan.keep_ready(resource));
                    }
                    return self.preload();
                }
            }
            Op::Next | Op::Prev => {
                let moved = if op == Op::Next {
                    engine.next()
                } else {
                    engine.prev()
                };
                // The operator moved the plan on: the step that was being
                // looked at lets go, so the row on stage is the only one
                // lit and Up next shows what follows. A step going on by
                // itself leaves the selection alone: it may be mid-edit.
                if matches!(moved, lantern_engine::Moved::Step(_))
                    && self.selected.take().is_some()
                    && self.search.picked().is_none()
                {
                    engine.clear_preview();
                }
                return Task::batch([self.preload(), self.arrive()]);
            }
            Op::PlayPause => {
                engine.toggle_playback();
            }
            Op::Blackout => {
                engine.toggle_blackout();
            }
            Op::Clear => engine.clear(),
            // Without a search the arrows are the plan pane's: the kinds
            // of step while Add step is open, the saved plans in the list,
            // the steps in a plan.
            Op::PickNext | Op::PickPrev if !self.searching() => {
                let down = op == Op::PickNext;
                let around = |at: Option<usize>, len: usize| match (at, down) {
                    _ if len == 0 => None,
                    (None, true) => Some(0),
                    (None, false) => Some(len - 1),
                    (Some(at), true) => Some((at + 1) % len),
                    (Some(at), false) => Some((at + len - 1) % len),
                };
                if let Some(kind) = self.adding {
                    self.adding = around(Some(kind), plan_edit::NewStep::ALL.len());
                } else if let plans::Mode::Browsing(entries) = &self.plans.mode {
                    let order = self.plans.listed(entries);
                    let at = self
                        .list_pick
                        .as_ref()
                        .and_then(|pick| order.iter().position(|entry| &entry.path == pick));
                    self.list_pick = around(at, order.len()).map(|at| order[at].path.clone());
                } else {
                    // The steps, then what's on standby under them.
                    let plan = engine.plan();
                    let steps = plan.len();
                    let at = match self.selected.and_then(|id| plan.position(id)) {
                        Some(at) => Some(at),
                        None => self.standby_lined_up().map(|at| steps + at),
                    };
                    match around(at, steps + plan.standby.len()) {
                        Some(at) if at < steps => {
                            return self.update(Message::Select(plan.steps()[at].id));
                        }
                        Some(at) => return self.line_up_standby(&plan.standby[at - steps].id),
                        None => {}
                    }
                }
            }
            Op::PickNext | Op::PickPrev => {
                if let Some(resource) = self.search.step(op == Op::PickNext) {
                    engine.preview(resource);
                    return self.scroll_to_pick();
                }
            }
            Op::ClearSearch if self.settings.is_some() => return self.op(Op::Settings),
            // Esc lets go of a drag without moving anything.
            Op::ClearSearch if self.drag.is_some() => self.drag = None,
            Op::ClearSearch => {
                // Esc also lets go of a name being typed, and of Add step.
                if matches!(self.plans.mode, plans::Mode::Renaming(_)) {
                    self.plans.cancel();
                }
                // With nothing else to let go of, Esc leaves the list of
                // plans for the plan that's open.
                let idle = self.adding.is_none()
                    && self.importing.is_none()
                    && !self.searching()
                    && !matches!(self.plans.mode, plans::Mode::Renaming(_));
                if idle
                    && matches!(self.plans.mode, plans::Mode::Browsing(_))
                    && self.plans.has_plan(&engine.plan())
                {
                    self.plans.cancel();
                }
                // Esc empties the search, and closes it once it's empty.
                if self.search_open && self.search.query.is_empty() && self.search.bang().is_none()
                {
                    self.search_open = false;
                }
                self.adding = None;
                self.editing = None;
                self.list_pick = None;
                self.importing = None;
                self.search.clear(&engine);
                return operation::focus(SEARCH_BOX);
            }
            Op::Save if self.settings.is_some() => self.save_settings(),
            // Closing with changes unsaved asks once: the first press says
            // so, the second discards them.
            Op::Settings
                if self
                    .settings
                    .as_ref()
                    .is_some_and(|draft| draft.is_dirty() && !draft.leaving) =>
            {
                keys::record(false);
                if let Some(draft) = &mut self.settings {
                    draft.leaving = true;
                    draft.notice = Some(Err(format!(
                        "Unsaved changes: {} again discards them.",
                        keys::label(Op::ClearSearch)
                    )));
                }
            }
            Op::Settings => {
                keys::record(false);
                self.starts_typed = None;
                self.deleting_plan = false;
                self.settings = match self.settings {
                    Some(_) => None,
                    None => Some(self.settings_draft()),
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
            Op::Search => {
                self.search_open = true;
                self.adding = None;
                return Task::batch([
                    operation::focus(SEARCH_BOX),
                    operation::move_cursor_to_end(SEARCH_BOX),
                ]);
            }
            Op::Rename => {
                let in_plan = !matches!(self.plans.mode, plans::Mode::Browsing(_));
                if let Some(step) = self.selected.filter(|_| in_plan && !self.searching()) {
                    return self.update(Message::EditStep(Some((step, plan_edit::Part::Name))));
                }
            }
            Op::RemoveStep => {
                if let Some(step) = self.selected.filter(|_| !self.searching()) {
                    return self.update(Message::Remove(step));
                }
                // With no step selected, what's lined up from standby.
                if let Some(at) = self.standby_lined_up().filter(|_| !self.searching()) {
                    let id = engine.edit_plan(|plan| plan.standby[at].id.clone());
                    return self.update(Message::Unstandby(id));
                }
            }
            // With no edit to take back, a plan just deleted comes back.
            Op::Undo if self.undo.is_empty() && self.plans.undelete(&engine) => {
                return self.switched(true);
            }
            Op::Undo => {
                if let Some(before) = self.undo.pop() {
                    self.editing = None;
                    self.typing = None;
                    self.drafts.clear();
                    engine.edit_plan(|plan| *plan = before);
                    let plan = engine.plan();
                    self.selected = self.selected.filter(|id| plan.get(*id).is_some());
                    return self.preload();
                }
            }
            Op::Open => {
                self.list_pick = None;
                self.adding = None;
                self.plans.browse();
            }
            // In the list of plans it's a new plan; in a plan, a new step.
            Op::New if matches!(self.plans.mode, plans::Mode::Browsing(_)) => {
                return self.switch_plan(Switch::New);
            }
            Op::New => return self.update(Message::ToggleAdd),
            Op::MoveUp | Op::MoveDown => {
                if let Some(step) = self.selected {
                    let by = if op == Op::MoveUp { -1 } else { 1 };
                    return self.update(Message::Move(step, by));
                }
            }
            Op::Complete if self.settings.is_some() => {
                return self.update(Message::Settings(settings::Edit::ChooseFirst));
            }
            Op::Complete => {
                if let Some(text) = self.search.completion(&engine) {
                    return self.complete(text);
                }
            }
        }
        Task::none()
    }

    /// The service may have reached a new step. Stepping on keeps the arrows
    /// stepping; a live slot opens the search with what it searches, ready
    /// for Enter.
    fn arrive(&mut self) -> Task<Message> {
        let current = self.engine.0.current_step();
        let id = current.as_ref().map(|step| step.id);
        if id == self.arrived_at {
            self.stepping = true;
            return Task::none();
        }
        self.arrived_at = id;
        let Some(Step {
            action: Action::Live { hint, .. },
            ..
        }) = current
        else {
            self.stepping = true;
            return Task::none();
        };
        self.search.clear(&self.engine.0);
        // A live slot is the time to search.
        self.search_open = true;
        let text = hint.map(|query| query.text).unwrap_or_default();
        if text.is_empty() {
            return operation::focus(SEARCH_BOX);
        }
        // The slot's bang becomes the chip, so a space ends it.
        let text = if text.contains(char::is_whitespace) || !text.starts_with('!') {
            text
        } else {
            format!("{text} ")
        };
        self.complete(text)
    }

    /// Writes the settings to the config file and applies them: the library
    /// swaps its sources and the colours follow the new file. What happened
    /// shows under the settings.
    /// Moves the stage to its screen when it first appears, when the
    /// screen asked for changes, or when that screen is plugged in.
    fn check_outputs(&mut self) {
        let wanted = self.config.stage_output.as_deref();
        let (Ok(connected), Ok(stage)) = (outputs::list(), outputs::stage_on()) else {
            return;
        };
        let names = connected.into_iter().map(|o| o.name);
        if self.output_watch.check(names, wanted, stage.is_some())
            && let Some(output) = wanted
            && stage.as_deref() != Some(output)
        {
            move_stage(output);
        }
    }

    /// The settings as saved, with the open plan's look too.
    fn settings_draft(&self) -> settings::Draft {
        let plan = self.engine.0.plan();
        let mut draft = settings::Draft::of(&self.config, self.home.as_deref())
            .with_plan(&plan)
            .with_outputs(outputs::list())
            .with_fonts(fit::families());
        draft.mark_saved();
        draft
    }

    fn save_settings(&mut self) {
        let Some(draft) = &mut self.settings else {
            return;
        };
        let path = self.config_status.path().to_path_buf();
        let home = self.home.as_deref();
        let dir = path.parent().unwrap_or(std::path::Path::new("."));
        // Both looks must read before anything is written.
        let plan_folders = draft.plan_folders();
        let plan_bibles = draft.plan_bibles();
        let saved = draft.plan_look(dir, home).and_then(|plan_look| {
            draft
                .config(dir, home)
                .and_then(|config| config.save(&path, home).map(|()| (config, plan_look)))
        });
        match saved {
            Ok((config, plan_look)) => {
                // Saved all the same: a folder may be on a stick that's out.
                let missing = draft.missing_folders(dir, home);
                draft.notice = Some(Ok(match missing.as_slice() {
                    [] => settings::saved(&path, home),
                    missing => format!(
                        "{} Not found, so not searched: {}.",
                        settings::saved(&path, home),
                        missing.join(", ")
                    ),
                }));
                draft.mark_saved();
                // A look just saved is meant to be seen.
                self.look_held = false;
                // The plan's own look goes into the plan, saved with it when
                // it has a file (an untitled plan shows Save instead).
                if let Some(look) = plan_look
                    && self.engine.0.plan().look != look
                {
                    self.engine.0.edit_plan(|plan| plan.look = look);
                    if self.plans.has_file() {
                        self.plans.save(&self.engine.0);
                    }
                }
                // So do its own folders and its Bibles.
                if let Some(folders) = plan_folders
                    && self.engine.0.plan().folders != folders
                {
                    self.engine.0.edit_plan(|plan| plan.folders = folders);
                    if self.plans.has_file() {
                        self.plans.save(&self.engine.0);
                    }
                }
                if let Some(bibles) = plan_bibles
                    && self.engine.0.plan().bibles != bibles
                {
                    self.engine.0.edit_plan(|plan| plan.bibles = bibles);
                    if self.plans.has_file() {
                        self.plans.save(&self.engine.0);
                    }
                }
                log(format!("settings: saved {}", path.display()));
                // Read fine when the draft became this config.
                let _ = keys::configure(&config.keys);
                let colors_changed = config.colors != self.config.colors;
                self.config = config.clone();
                self.plan_needs = library::PlanNeeds::of(&self.engine.0.plan());
                self.library
                    .0
                    .apply(self.library_config(&self.plan_needs.clone()));
                if colors_changed {
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

    /// Keeps the plan as it is, before an edit, for Ctrl+Z. Keystrokes in
    /// one step's fields (`typing`) are kept once, so they undo together.
    fn remember(&mut self, typing: Option<StepId>) {
        if typing.is_some() && typing == self.typing {
            return;
        }
        self.typing = typing;
        if self.undo.len() == UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.undo.push(self.engine.0.plan());
    }

    /// Replaces the plan with `to`, asking first while the plan is running.
    fn switch_plan(&mut self, to: Switch) -> Task<Message> {
        if self.engine.0.current_step().is_some() {
            self.plans.ask_to_leave(to);
            return Task::none();
        }
        let replaced = self.plans.switch(&self.engine.0, to);
        self.switched(replaced)
    }

    /// After a plan may have replaced the last one: preloads for it, and a
    /// brand new plan opens on its name, so it isn't left "Untitled".
    fn switched(&mut self, replaced: bool) -> Task<Message> {
        let preload = self.preload_if(replaced);
        if replaced && self.plans.is_blank(&self.engine.0.plan()) {
            self.plans.edit_name(&self.engine.0);
            return Task::batch([
                preload,
                operation::focus(NAME_BOX),
                operation::select_all(NAME_BOX),
            ]);
        }
        preload
    }

    /// Keeps the side panes' widths for the next start.
    fn keep_layout(&self) {
        let kept = self
            .plans
            .store()
            .set_layout(self.plan_dragged, self.stage_dragged);
        if let Err(err) = kept {
            log(format!("layout: {err}"));
        }
    }

    /// Whether a search is under way: the arrows and Enter are its, and
    /// otherwise the plan pane's.
    pub(crate) fn searching(&self) -> bool {
        self.search_open
    }

    /// Preloads for a plan that replaced the last one.
    fn preload_if(&mut self, replaced: bool) -> Task<Message> {
        if replaced {
            // What the audience is looking at doesn't change its font and
            // colours because the operator opened another plan.
            self.look_held = self.stage.program.is_some();
            // Ctrl+Z never reaches back into the plan that was open before.
            self.editing = None;
            self.undo.clear();
            self.typing = None;
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
            StageEvent::Program { .. } => {
                // Something else is up, or nothing: the open plan's look.
                self.look_held = false;
                self.report_slides();
                return self.program_changed();
            }
            StageEvent::Preview { .. } => {
                self.report_slides();
                return self.preload();
            }
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

    /// Tells the stage how many slides the texts on program and in preview
    /// need with the current look and stage shape, so stepping goes through
    /// every piece of a text too long for one slide.
    fn report_slides(&self) {
        for scene in [&self.stage.program, &self.stage.preview]
            .into_iter()
            .flatten()
        {
            let Some(body) = scene.resource.location.as_text() else {
                continue;
            };
            let Some(count) = std::num::NonZeroUsize::new(self.slides.slides(body).len()) else {
                continue;
            };
            let known =
                matches!(scene.media, Media::Frames { count: Some(known), .. } if known == count);
            if !known {
                self.engine.0.report(scene.id, MediaReport::Frames(count));
            }
        }
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
    /// How long until the current step goes on by itself, for a still
    /// step (an image, page, slide or black) that does: its length, shared
    /// among its slides, counted from when the current slide went up. A
    /// video goes on when it ends, which the engine does.
    pub(crate) fn goes_on_in(&self) -> Option<Duration> {
        let step = self.engine.0.current_step().filter(|step| step.advance)?;
        if !matches!(step.action, Action::Show { .. } | Action::Blank) {
            return None;
        }
        let program = self.engine.0.stage().program;
        if program
            .as_ref()
            .is_some_and(|scene| scene.timed().is_some())
        {
            return None;
        }
        let (scene, frame, frames) = match &program {
            Some(scene) => (Some(scene.id), scene.frame(), frame_count(scene)),
            None => (None, 0, 1),
        };
        let since = match self.shown_since {
            Some((s, f, since)) if (s, f) == (scene, frame) => since,
            _ => Instant::now(),
        };
        let each = self.estimate(&step).length / u32::try_from(frames).unwrap_or(1).max(1);
        Some(each.saturating_sub(since.elapsed()))
    }

    /// Notes when what's on stage (scene or slide) changed.
    fn mark_shown(&mut self) {
        let program = self.engine.0.stage().program;
        let shown = (
            program.as_ref().map(|s| s.id),
            program.as_ref().map_or(0, |s| s.frame()),
        );
        if self.shown_since.is_none_or(|(s, f, _)| (s, f) != shown) {
            self.shown_since = Some((shown.0, shown.1, Instant::now()));
        }
    }

    /// On each tick of a running service: moves a still step that goes on
    /// by itself once its time is up.
    fn go_on(&mut self) -> Task<Message> {
        self.mark_shown();
        if self.goes_on_in().is_some_and(|left| left.is_zero()) {
            return self.op(Op::Next);
        }
        Task::none()
    }

    /// Points a moving drag at the row half under the pointer. Over the
    /// gap (no row) it keeps aiming where it was, so the gap stays put.
    fn aim(&mut self) {
        let Some(drag) = self.drag.as_mut().filter(|drag| drag.moving) else {
            return;
        };
        let plan = self.engine.0.plan();
        if let Some(landing) = self
            .hovered
            .and_then(|(over, half)| drag.landing(&plan, over, half))
        {
            drag.landing = Some(landing);
        }
    }

    /// Lines up something from the plan's standby shelf: it's looked at in
    /// the preview, and Enter (or a second click) shows it. No step stays
    /// selected, so Enter means this.
    fn line_up_standby(&mut self, id: &ResourceId) -> Task<Message> {
        let resource = self
            .engine
            .0
            .edit_plan(|plan| plan.standby.iter().find(|r| &r.id == id).cloned());
        if let Some(resource) = resource {
            self.selected = None;
            self.editing = None;
            self.search.unpick();
            self.engine.0.preview(Arc::new(resource));
        }
        self.preload()
    }

    /// Which of the standby shelf is lined up in the preview, while no
    /// step is selected: where the arrows are, under the steps.
    pub(crate) fn standby_lined_up(&self) -> Option<usize> {
        if self.selected.is_some() {
            return None;
        }
        let preview = self.stage.preview.as_ref()?;
        self.engine.0.edit_plan(|plan| {
            plan.standby
                .iter()
                .position(|r| r.id == preview.resource.id)
        })
    }

    /// The config as the open plan needs it indexed: its own folders
    /// too, and only its Bibles.
    fn library_config(&self, needs: &library::PlanNeeds) -> Config {
        let file = self.config_status.path();
        let dir = file.parent().unwrap_or(std::path::Path::new("."));
        needs.apply(self.config.clone(), dir, self.home.as_deref())
    }

    /// Gives the library what the open plan needs when that isn't what it
    /// has: another plan was opened, or its folders or Bibles were edited.
    /// The index is built again, so only on a change.
    fn sync_library(&mut self) {
        let needs = self.engine.0.edit_plan(|plan| library::PlanNeeds::of(plan));
        if needs != self.plan_needs {
            log(format!(
                "library: the plan has {} folders of its own and {}",
                needs.folders.len(),
                match needs.bibles.len() {
                    0 => "every Bible".to_string(),
                    n => format!("{n} of the Bibles"),
                }
            ));
            self.library.0.apply(self.library_config(&needs));
            self.plan_needs = needs;
        }
    }

    fn preload(&mut self) -> Task<Message> {
        // Every change of plan comes through here.
        self.sync_library();
        let plan = self.engine.0.plan();
        let shown = plan.steps().iter().filter_map(|step| match &step.action {
            Action::Show { resource } => Some(resource),
            _ => None,
        });
        // What's on standby must be as ready as the steps. The plan's
        // videos are downloaded; one shown outside it is streamed.
        let planned: Vec<YoutubeVideo> = shown
            .chain(&plan.standby)
            .filter_map(YoutubeVideo::of)
            .collect();
        for video in &planned {
            self.downloads.0.fetch(video);
        }
        self.media.plan_videos(
            planned.into_iter().map(|video| video.id).collect(),
            !self.config.always_download,
        );
        let preview = self.stage.preview.as_ref().map(|s| (*s.resource).clone());
        let next = self.engine.0.up_next().and_then(|step| match step.action {
            Action::Show { resource } => Some(resource),
            _ => None,
        });
        // The plan's pictures and pages stay loaded while it's open: the
        // shelf first, which is shown without warning, then the steps.
        let stills = plan
            .standby
            .iter()
            .cloned()
            .chain(plan.steps().iter().filter_map(|step| match &step.action {
                Action::Show { resource } => Some(resource.clone()),
                _ => None,
            }));
        let keep = self.media.keep(stills).map(Message::Loaded);
        let upcoming = self
            .media
            .preload(preview.into_iter().chain(next))
            .map(Message::Loaded);
        Task::batch([keep, upcoming])
    }

    fn view(&self, window: window::Id) -> Element<'_, Message> {
        if window == self.stage_window {
            self.stage_view()
        } else {
            self.dashboard_view()
        }
    }

    /// The look's background, with the program on top unless it failed.
    /// Blackout covers the program instead of removing it, so a video
    /// keeps playing and still reports its end; it's black whatever the
    /// look.
    fn stage_view(&self) -> Element<'_, Message> {
        let look = &self.look;
        let Some(scene) = &self.stage.program else {
            // Blackout is black even with nothing on: not the look's image.
            if self.stage.blackout {
                return black(Space::new());
            }
            return backdrop(look, Space::new(), Over::Nothing);
        };
        let nothing = || (Space::new().into(), Over::Nothing);
        let (content, over): (Element<'_, Message>, Over) =
            match (&scene.media, self.slides.slide(scene)) {
                (Media::Failed { .. }, _) => nothing(),
                (_, Some(body)) => (fit::fit_text(body, look).into(), Over::Text),
                (_, None) => match self.media.content(scene.id) {
                    None => nothing(),
                    Some(Content::Image(handle)) => (fit(image(handle.clone())), Over::Media),
                    Some(Content::Pages(pages)) => {
                        let page = &pages[scene.frame().min(pages.len() - 1)];
                        (fit(image(page.clone())), Over::Media)
                    }
                    Some(Content::Video { player, .. }) => (
                        VideoPlayer::new(player)
                            .width(Fill)
                            .height(Fill)
                            .content_fit(ContentFit::Contain)
                            .on_end_of_stream(Message::Ended(scene.id))
                            .into(),
                        Over::Media,
                    ),
                },
            };
        if self.stage.blackout {
            stack![backdrop(look, content, over), black(Space::new())].into()
        } else {
            backdrop(look, content, over)
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            Subscription::run_with(self.engine.clone(), stage_feed),
            window::close_events().map(Message::Closed),
            window::resize_events().map(|(window, size)| Message::Resized(window, size)),
            iced::event::listen_with(keys::key_event),
            iced::event::listen_with(pointer_event),
        ];
        // The pointer is followed only while it holds a row or a pane's edge.
        if self.drag.is_some() || self.resizing.is_some() {
            subscriptions.push(iced::event::listen_with(cursor_event));
        }
        // A search on screen follows files added to or removed from a folder.
        if !self.search.query.is_empty() {
            subscriptions
                .push(iced::time::every(Duration::from_secs(1)).map(|_| Message::CheckIndex));
        }
        // The stage's screen, when one is asked for: at startup, and when
        // it's plugged in later.
        if self.config.stage_output.is_some() && outputs::supported() {
            subscriptions
                .push(iced::time::every(Duration::from_secs(2)).map(|_| Message::CheckOutputs));
        }
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
        // Before it, a plan with a start time counts down to it.
        else if self.engine.0.edit_plan(|plan| plan.starts.is_some()) {
            subscriptions
                .push(iced::time::every(Duration::from_secs(15)).map(|_| Message::Progress));
        }
        // A step counting down to going on checks often, so it's on time.
        if self.goes_on_in().is_some() {
            subscriptions
                .push(iced::time::every(Duration::from_millis(250)).map(|_| Message::Progress));
        }
        if self.downloads.0.busy() || self.library.0.indexing() || self.thumbs.busy() {
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
/// What was typed into `old` to make `new`: the text between what the two
/// share at the start and at the end. `None` when nothing was added to a
/// box that had text, as when deleting.
fn typed_into(old: &str, new: &str) -> Option<String> {
    if old.is_empty() || new.len() <= old.len() {
        return None;
    }
    let start: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let end: usize = old[start..]
        .chars()
        .rev()
        .zip(new[start..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    (start + end == old.len()).then(|| new[start..new.len() - end].to_string())
}

fn set_length(plan: &mut Plan, id: StepId, value: Option<Duration>) {
    let Some(step) = plan.get_mut(id) else { return };
    match &mut step.action {
        Action::Section { budget } => *budget = value,
        _ => step.estimate = value,
    }
}

/// What a backdrop is behind, which decides how much of it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Over {
    /// Nothing that needs reading: the image as it is.
    Nothing,
    /// Slide text: the image dimmed by the look's `dim`, so it reads.
    Text,
    /// A picture, page or video: only the colour, so what its shape leaves
    /// uncovered is plain bars, not strips of the look's image.
    Media,
}

/// `content` over the look's background: its colour, and its image
/// covering the stage, dimmed toward the colour under text.
pub(crate) fn backdrop<'a>(
    look: &Look,
    content: impl Into<Element<'a, Message>>,
    over: Over,
) -> Element<'a, Message> {
    let colour = fit::color(look.background);
    let content = container(content).center(Fill);
    let layers: Element<'a, Message> = match look.image.as_ref().filter(|_| over != Over::Media) {
        Some(path) => {
            let dimming = match over {
                Over::Nothing | Over::Media => 0.0,
                Over::Text => look.dimming(),
            };
            let dim = container(Space::new())
                .width(Fill)
                .height(Fill)
                .style(move |_| {
                    container::Style::default().background(colour.scale_alpha(dimming))
                });
            stack![
                image(image::Handle::from_path(path))
                    .width(Fill)
                    .height(Fill)
                    .content_fit(ContentFit::Cover),
                dim,
                content,
            ]
            .into()
        }
        None => content.into(),
    };
    container(layers)
        .center(Fill)
        .clip(true)
        .style(move |_| container::Style::default().background(colour))
        .into()
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

/// The left button going down or up, for dragging plan rows. Every press
/// counts, even one a button took: the row's button takes it.
fn pointer_event(
    event: iced::Event,
    _: iced::event::Status,
    window: window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)) => {
            Some(Message::Pointer(Some(window), true))
        }
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
            Some(Message::Pointer(Some(window), false))
        }
        _ => None,
    }
}

/// Where the pointer moved, while a plan row or a pane's edge is held.
fn cursor_event(event: iced::Event, _: iced::event::Status, window: window::Id) -> Option<Message> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
            Some(Message::Cursor(Some(window), position))
        }
        _ => None,
    }
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
/// Runs `work` on a thread of its own, so copying files doesn't freeze
/// the windows.
fn in_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> impl Future<Output = Result<T, String>> {
    let (done, result) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = done.send(work());
    });
    async move {
        result
            .await
            .unwrap_or_else(|_| Err("it stopped before it finished".into()))
    }
}

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

/// How many frames a scene shows: its pages or slides, 1 when not known.
fn frame_count(scene: &lantern_engine::Scene) -> usize {
    match &scene.media {
        Media::Frames {
            count: Some(count), ..
        } => count.get(),
        _ => 1,
    }
}

/// Puts the stage on `output` and logs where it ended up, read back from
/// the compositor.
fn move_stage(output: &str) {
    match outputs::put_stage_on(output).and_then(|()| outputs::stage_on()) {
        Ok(on) => log(format!(
            "outputs: asked for {output}, the stage is on {}",
            on.as_deref().unwrap_or("no screen yet")
        )),
        Err(why) => log(format!("outputs: can't put the stage on {output}: {why}")),
    }
}

#[cfg(test)]
mod tests {
    use super::typed_into;

    #[test]
    fn typing_after_a_show_keeps_only_what_was_typed() {
        let typed = |old, new| typed_into(old, new);
        assert_eq!(typed("jn 3 16", "jn 3 16s").as_deref(), Some("s"));
        assert_eq!(typed("jn 3 16", "sjn 3 16").as_deref(), Some("s"));
        assert_eq!(typed("jn 3 16", "jn sal 3 16").as_deref(), Some("sal "));
        assert_eq!(typed("amén", "aménñ").as_deref(), Some("ñ"));
        assert_eq!(typed("aa", "aaa").as_deref(), Some("a"));
        assert_eq!(
            typed("jn 3 16", "jn 3 1"),
            None,
            "deleting edits the old search"
        );
        assert_eq!(typed("", "s"), None, "an empty box has nothing to replace");
        assert_eq!(typed("jn 3", "salmo 23"), None, "replaced, not added to");
    }
}
