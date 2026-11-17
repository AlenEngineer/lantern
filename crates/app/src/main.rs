//! The lantern desktop app: an operator dashboard and a fullscreen stage
//! window for the projector, both following one [`Engine`].
//!
//! It reads its media folders and Bibles from a config file (see
//! [`config::load`]) and indexes them in the background.
//!
//! Set `LANTERN_SCRIPT=<file>` to drive the engine from a script and exit
//! after its last line (see [`script`]). `LANTERN_SHOT=<program>` is run
//! for each `shot NAME` in it.

mod before;
mod bundle;
mod config;
mod cut;
mod dashboard;
mod deck;
mod desktop;
mod drag;
mod fit;
mod keys;
mod length;
mod library;
mod look;
mod mode;
mod motion;
mod outputs;
mod palette;
mod paths;
mod phone;
mod plan_edit;
mod plan_pane;
mod plans;
mod ready;
mod rehearse;
mod relay;
mod script;
mod search;
mod settings;
mod shelf;
mod side;
mod stage;
mod thumbs;
mod watch;
mod youtube;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
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

/// How long a close of the dashboard, asked for mid-service, waits to be
/// asked again.
const SURE_QUIT: Duration = Duration::from_secs(4);

/// How long after a letter typed into a closed search Space is taken for
/// part of what was being typed, and not for "go on".
const STRAY: Duration = Duration::from_millis(1500);
/// How many plan edits Ctrl+Z can take back.
const UNDO_DEPTH: usize = 50;

/// Where an arrow, Home or End takes a list's selection.
fn go(op: Op) -> shelf::Go {
    match op {
        Op::PickPrev => shelf::Go::Prev,
        Op::PickFirst => shelf::Go::First,
        Op::PickLast => shelf::Go::Last,
        _ => shelf::Go::Next,
    }
}

/// What a click lands on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Clicked {
    Result(ResourceId),
    Step(StepId),
    /// Something on the plan's standby shelf.
    Standby(ResourceId),
    /// A file in the library tree.
    Shelf(ResourceId),
}

#[derive(Debug, Clone)]
enum Message {
    /// The stage as it was when the feed started. Always the feed's first message.
    Feed(StageState),
    Stage(StageEvent),
    Loaded(Arc<Loaded>),
    /// The program's video reached its end.
    Ended(SceneId),
    /// A phone asked for something through the remote.
    Remote(lantern_remote::Command),
    /// Time to take a picture of the stage for the phones showing it.
    Snap,
    /// The picture, as a JPEG, or why there is none.
    Snapped(Result<Arc<Vec<u8>>, String>),
    /// The program's video stopped on an error, which this says.
    Broke(SceneId, String),
    /// Time to report the playing video's position.
    Tick,
    /// Time to redraw download or indexing progress.
    Progress,
    /// Time to draw what is moving a little further along.
    Frame,
    /// The script performed a command. The plan may have changed, which the
    /// stage feed doesn't announce.
    Acted,
    ScriptDone,
    /// A dashboard command from the script.
    Script(Ui),
    Closed(window::Id),
    /// A window was asked to close: by its button, or by the desktop.
    CloseAsked(window::Id),
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
    /// A key that types while the search is open and acts once it is
    /// closed: Space, which goes on to what is next.
    KeyIfClosed {
        op: Op,
        typed: String,
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
    /// Rehearse a section, or the plan (`None`), in the preview; or stop.
    Rehearse(Option<StepId>),
    /// The rehearsal goes on to its next step now.
    RehearseNext,
    /// The rehearsal's clock stops, or runs again.
    RehearsePause,
    /// The rehearsal's clock runs at its next speed.
    RehearseSpeed,
    /// The rehearsal's clock ticked.
    RehearseTick,
    /// What the side panel shows.
    Lens(side::Lens),
    /// A value of the selected thing's look, as typed.
    LookEdit(side::Field, String),
    /// That value goes back to what the level above gives.
    LookReset(side::Field),
    /// Nothing in the plan is selected: the plan itself is in hand.
    Unselect,
    /// The plan's target time is to be typed.
    EditTarget,
    /// How long the whole service is meant to take, as typed.
    PlanTarget(String),
    /// The target is typed.
    TargetDone,
    /// Whether the plan's steps go on by themselves unless one says.
    PlanAdvance(bool),
    /// How many times a step or section comes, or the plan (`None`).
    SetRepeat(Option<StepId>, lantern_engine::Repeat),
    /// Whether a section's steps, or the plan's, come in a drawn order.
    SetShuffle(Option<StepId>, bool),
    /// Opens the choices of how often it comes, or closes them.
    RepeatPanel(Option<StepId>),
    /// The time of day a repeat ends at, as typed.
    RepeatUntil(String),
    /// The typed time is the one: the repeat ends then.
    RepeatUntilSet,
    /// Gives a step a sound to play with it, or takes it away.
    StepSound(StepId, Option<Arc<lantern_engine::Resource>>),
    /// Copies what a step shows, a file outside the media folders, to
    /// where Lantern keeps plans' media, so the plan has it from then on.
    KeepCopy(StepId),
    /// The copy is made, and where; or why it couldn't be.
    KeptCopy(StepId, Result<PathBuf, String>),
    /// The slide a step's sound starts from, as typed: counted from 1.
    SoundFrom(StepId, String),
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
    /// Shows what the slide on stage plays or links to: the Nth of them.
    Attachment(usize),
    /// Back to the deck that was left for one of its attachments: at the
    /// slide after the one it was left at, or at that one.
    BackToSlides {
        next: bool,
    },
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
    /// Puts what is picked in a step's place.
    ReplaceStep(StepId),
    /// A line inside a step was clicked: go to the slide it is on. In a
    /// run, on the projector; while planning, in the preview.
    StepSlide(StepId, usize),
    /// Folds a section of the plan away, or opens it again.
    Fold(StepId),
    /// The section goes into the one before it (`true`), or out a level.
    Nest(StepId, bool),
    /// Opens a folder of the library tree, or closes it.
    ShelfFold(String),
    /// What is typed in the library's filter.
    ShelfFilter(String),
    /// The pointer is over this file of the library, or over none.
    ShelfHover(Option<ResourceId>),
    /// The Plan or Live button of the mode switch.
    SetMode(mode::Mode),
    /// From planning to running the plan.
    StartService,
    /// The look round said something, and the service is not started.
    NotYet,
    /// From running back to planning: asked twice.
    EndService,
    /// Makes a plan of what was shown in Live.
    SaveSession,
}

/// A deck left for something one of its slides plays or links to.
pub(crate) struct Left {
    deck: Arc<lantern_engine::Resource>,
    /// The slide it was at, counted from 0.
    pub(crate) frame: usize,
    /// What it was left for.
    pub(crate) shown: lantern_engine::ResourceId,
    /// Whether the deck has a slide after that one, for Next to go on to.
    pub(crate) more: bool,
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
    /// The remote for a phone, while it is on.
    remote: Option<phone::Remote>,
    /// Why the remote couldn't start, when it couldn't.
    remote_failed: Option<String>,
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
    /// Where the stage was last put by hand, for each set of screens.
    screens: outputs::Memory,
    /// Where the stage goes with the screens connected now; `None` until
    /// they have been listed, or where they can't be.
    place: Option<outputs::Place>,
    /// The screens connected at the last look, as one name.
    setup: String,
    /// The screen the stage window was on at the last look.
    stage_seen_on: Option<String>,
    /// The stage's screen went away with a service or Live going.
    pub(crate) stage_lost: bool,
    /// The library's change count the search last followed.
    index_changes: u64,
    /// Thumbnails for results and plan steps.
    thumbs: thumbs::Thumbs,
    /// What the deck on stage plays or links to that the stage can show,
    /// a list for each of its slides.
    pub(crate) attached: Option<(
        lantern_engine::ResourceId,
        Vec<Vec<Arc<lantern_engine::Resource>>>,
    )>,
    /// The deck that was left to show one of its attachments.
    pub(crate) left: Option<Left>,
    /// The roles file the palette follows, if the config names one.
    colors: Option<palette::Source>,
    /// Why the roles file couldn't be read, while it can't.
    palette_error: Option<String>,
    script: Option<Script>,
    dashboard_window: window::Id,
    /// The stage window, while it is open: with a service or in Live, or
    /// opened by hand.
    pub(crate) stage_window: Option<window::Id>,
    /// The operator opened or closed the stage by hand: it stays so until
    /// the mode changes, whatever the mode would have it be.
    stage_wish: Option<bool>,
    /// The mode the stage window was last matched to.
    stage_mode: mode::Mode,
    /// When the dashboard was asked to close with the projector in use:
    /// asked again soon after, it closes.
    quitting: Option<Instant>,
    /// What is held from the desktop: the screens on, notifications off.
    desktop: desktop::Desktop,
    /// Lantern is closing.
    leaving: bool,
    /// The dashboard window's size, to lay out for the room there is.
    dashboard_size: iced::Size,
    /// The stage window's mirror, built only from the engine's feed.
    stage: StageState,
    media: Stage,
    search: Search,
    /// The search a phone asked for through the remote: its own, so it
    /// neither shows in the dashboard's box nor disturbs what is typed
    /// there. What it was sent is kept to say back with the results.
    phone_search: Search,
    phone_asked: String,
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
    /// When a letter was last typed with the search closed, where it goes
    /// nowhere.
    stray: Option<Instant>,
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
    /// The plan's target time as typed, while its field is open.
    pub(crate) target_typed: Option<String>,
    /// What the side panel shows while planning.
    /// The rehearsal under way.
    pub(crate) rehearsal: Option<rehearse::Rehearsal>,
    /// The rehearsal just over: of what, and how long it took.
    pub(crate) rehearsed: Option<(Option<StepId>, Duration)>,
    pub(crate) lens: side::Lens,
    /// A look value as typed so far: whose, which, and the text.
    pub(crate) look_typed: Option<(Option<StepId>, side::Field, String)>,
    /// Where sound went when the Check lens was last opened.
    pub(crate) sound_now: Option<before::Sound>,
    /// Delete was pressed once in the plan's settings: again deletes.
    pub(crate) deleting_plan: bool,
    /// The part of a step being edited in its row.
    editing: Option<(StepId, plan_edit::Part)>,
    /// The saved plan the arrows are on in the list of plans, for Enter.
    list_pick: Option<PathBuf>,
    /// The plan before each edit, newest last, for Ctrl+Z.
    undo: Vec<Plan>,
    /// The plans Ctrl+Z took back from, newest last, for doing again.
    /// A new edit empties it.
    redo: Vec<Plan>,
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
    /// Whose choices of how often it comes are open: a step or section,
    /// or the plan (`Some(None)`).
    pub(crate) repeat_panel: Option<Option<StepId>>,
    /// The time a repeat ends at, as typed so far.
    pub(crate) repeat_until: String,
    /// The slide a step's sound starts at, as it is being typed: it may
    /// be empty for a moment, on the way from one number to another.
    pub(crate) from_typed: Option<(StepId, String)>,
    /// What Lantern is being used for: planning, running a plan, or
    /// showing with none.
    pub(crate) mode: mode::Mode,
    /// The running plan is unlocked for a change, and the step the service
    /// was at when it was: it locks again when the service moves on.
    pub(crate) unlocked: Option<Option<StepId>>,
    /// "End the service" was pressed once, and when.
    pub(crate) ending: Option<Instant>,
    /// A service was running when Lantern last stopped: the step and the
    /// slide it was at, until it is picked up there or ended.
    pub(crate) resuming: Option<(StepId, usize)>,
    /// What the look round found as the service was asked to start: it
    /// starts on the next ask, or not at all.
    pub(crate) starting: Option<Vec<String>>,
    /// Where sound went as the last service started, for each set of
    /// connected screens.
    pub(crate) last_sound: outputs::Memory,
    /// The place of the running service as it was last written down.
    kept: Option<plans::Running>,
    /// What the decks in the plan play or link to, a list for each slide,
    /// by the deck's resource: read once, when the deck joins the plan.
    pub(crate) parts: HashMap<ResourceId, Vec<Vec<Arc<lantern_engine::Resource>>>>,
    /// The library's folders and their files, for its tree, and the
    /// state of the index they were read at.
    pub(crate) shelf: Vec<shelf::Folder>,
    pub(crate) shelf_read: Option<u64>,
    /// The folder of the tree that is open, and the file being looked at.
    pub(crate) shelf_open: std::collections::HashSet<String>,
    /// What is typed in the library's filter.
    pub(crate) shelf_filter: String,
    /// The file of the library under the pointer, and the one it holds.
    shelf_hover: Option<ResourceId>,
    /// The folder of the library the keyboard is on, when it is on one
    /// and not on a file.
    pub(crate) shelf_folder: Option<String>,
    /// The arrows are the library's, not the plan's.
    pub(crate) in_library: bool,
    pub(crate) carried: Option<shelf::Carried>,
    pub(crate) shelf_pick: Option<ResourceId>,
    /// The sections of the plan that are folded away.
    pub(crate) folded: std::collections::HashSet<StepId>,
    /// How much had been shown when Live was entered: its session is what
    /// was shown since.
    pub(crate) live_from: usize,
    /// Where the timeline is being dragged, until it is let go.
    scrub: Option<f64>,
    /// A row was pressed that the running plan won't let move: where,
    /// and whether the pointer has pulled at it since.
    pub(crate) balk: Option<(Option<iced::Point>, bool)>,
    /// The plan's file and its steps with what each shows, as last looked
    /// at: a step that is new or shows something else flashes.
    seen: (Option<PathBuf>, HashMap<StepId, Option<ResourceId>>),
    /// What is moving for a moment, to say something happened.
    pub(crate) motion: motion::Motion,
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
            // Asked first: with a service in front of people, one stray
            // close must not empty the projector.
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        // The stage feed's first message preloads the reopened plan.
        let layout = store.layout();
        let screens = outputs::Memory::new(&store.data());
        let last_sound = outputs::Memory::named(&store.data(), "sound-output");
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
            remote: None,
            remote_failed: None,
            dashboard_open: false,
            output_watch: outputs::Watch::default(),
            screens,
            place: None,
            setup: String::new(),
            stage_seen_on: None,
            stage_lost: false,
            index_changes: 0,
            thumbs: thumbs::Thumbs::start(),
            attached: None,
            left: None,
            config,
            settings: None,
            home: config::env("HOME").map(PathBuf::from),
            palette_error: None,
            downloads,
            script,
            dashboard_window,
            stage_window: None,
            stage_wish: None,
            stage_mode: mode::Mode::Plan,
            quitting: None,
            desktop: desktop::Desktop::default(),
            leaving: false,
            dashboard_size: iced::Size::new(1280.0, 720.0),
            stage: StageState::default(),
            search: Search::default(),
            phone_search: Search::default(),
            phone_asked: String::new(),
            selected: None,
            last_click: None,
            hovered: None,
            drag: None,
            stepping: false,
            stray: None,
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
            target_typed: None,
            rehearsal: None,
            rehearsed: None,
            lens: side::Lens::default(),
            look_typed: None,
            sound_now: None,
            deleting_plan: false,
            editing: None,
            list_pick: None,
            undo: Vec::new(),
            redo: Vec::new(),
            typing: None,
            arrived_at: None,
            shown_since: None,
            drafts: HashMap::new(),
            repeat_panel: None,
            repeat_until: String::new(),
            from_typed: None,
            mode: mode::Mode::Plan,
            unlocked: None,
            ending: None,
            resuming: None,
            starting: None,
            last_sound,
            kept: None,
            live_from: 0,
            folded: std::collections::HashSet::new(),
            parts: HashMap::new(),
            shelf: Vec::new(),
            shelf_read: None,
            shelf_open: std::collections::HashSet::new(),
            shelf_filter: String::new(),
            shelf_hover: None,
            shelf_folder: None,
            in_library: false,
            carried: None,
            shelf_pick: None,
            scrub: None,
            balk: None,
            seen: (None, HashMap::new()),
            motion: motion::Motion::new(),
        };
        app.motion.still = app.config.no_motion;
        app.check_palette();
        app.find_running();
        // The plan reopened at start may bring folders of its own.
        app.sync_library();
        // Only the dashboard: the stage opens when something can reach
        // it. A service picked up from before this start is one already.
        app.stage_mode = app.mode;
        app.check_outputs();
        app.sync_desktop();
        let stage = app.sync_stage();
        let open = open_dashboard
            .discard()
            .chain(stage)
            .chain(window::gain_focus(dashboard_window))
            .chain(operation::focus(SEARCH_BOX))
            .chain(
                window::size(dashboard_window)
                    .map(move |size| Message::Resized(dashboard_window, size)),
            );
        (app, open)
    }

    /// Asks of the desktop what the moment needs. The screens stay on
    /// with a service running or anything on the projector: a still slide
    /// through a sermon is no reason to turn them off. Notifications wait
    /// for the length of a service.
    fn sync_desktop(&mut self) {
        // Closing, with the service still running: what was taken back
        // is not asked for again.
        if self.leaving {
            return;
        }
        let running = self.mode == mode::Mode::Run;
        self.desktop
            .keep_awake(running || self.stage.program.is_some());
        if self.desktop.keep_quiet(running) {
            let command = match running {
                true => &self.config.on_service_start,
                false => &self.config.on_service_end,
            };
            if let Some(command) = command {
                desktop::run(command);
            }
        }
    }

    /// Lantern is closing: what a running service asked of the desktop
    /// by a command is taken back by its other command. What was asked
    /// over the bus ends with the connection.
    fn leave(&mut self) -> Task<Message> {
        self.leaving = true;
        self.desktop.keep_awake(false);
        if self.desktop.keep_quiet(false)
            && let Some(command) = &self.config.on_service_end
        {
            desktop::run(command);
        }
        iced::exit()
    }

    /// Opens or closes the stage window to match the mode: open with a
    /// service or in Live, closed while planning. What the operator did
    /// by hand holds until the mode changes.
    fn sync_stage(&mut self) -> Task<Message> {
        if self.mode != self.stage_mode {
            self.stage_mode = self.mode;
            self.stage_wish = None;
            self.stage_lost = false;
        }
        let wanted = self.stage_wish.unwrap_or(self.stage_expected());
        match (wanted, self.stage_window) {
            (true, None) => {
                // The dashboard is mapped first: on a single monitor,
                // Hyprland drops a fullscreen window back to tiled when
                // another window maps on its workspace. Then the keyboard
                // goes back to the dashboard.
                let (stage, open) = window::open(window::Settings {
                    fullscreen: true,
                    decorations: false,
                    platform_specific: platform(STAGE_APP_ID),
                    ..window::Settings::default()
                });
                self.stage_window = Some(stage);
                self.stage_lost = false;
                log("stage: the window opens");
                // Once it is there, it is put on its screen at once, not
                // at the next look round.
                open.discard()
                    .chain(window::gain_focus(self.dashboard_window))
                    .chain(window::size(stage).map(move |size| Message::Resized(stage, size)))
                    .chain(Task::done(Message::CheckOutputs))
            }
            (false, Some(stage)) => {
                self.stage_window = None;
                // Closed because its screen went, mid-service: that is
                // worth an alarm, where closing it for planning is not.
                self.stage_lost = self.stage_wish.is_none() && self.mode != mode::Mode::Plan;
                log("stage: the window closes");
                window::close(stage)
            }
            _ => Task::none(),
        }
    }

    /// Whether the stage window should be open, by the mode and the
    /// screens: on the preferred screen whenever that is connected; on
    /// another screen with a service or in Live; and not at all with
    /// nowhere to put it. Where the screens can't be listed, the mode
    /// alone decides.
    pub(crate) fn stage_expected(&self) -> bool {
        let showing = self.mode != mode::Mode::Plan;
        match &self.place {
            Some(outputs::Place::Preferred(_)) => true,
            Some(outputs::Place::Remembered(_) | outputs::Place::Spare(_)) => showing,
            Some(outputs::Place::Nowhere) => false,
            None => showing,
        }
    }

    fn title(&self, window: window::Id) -> String {
        if Some(window) == self.stage_window {
            "lantern stage".into()
        } else {
            "lantern".into()
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        // A repeat that ends at a time of day needs the time, whatever
        // moves the plan on.
        self.engine.0.tell_time(length::now_of_day());
        // A frame changes nothing: the view is drawn again, further along.
        if matches!(message, Message::Frame) {
            return Task::none();
        }
        if matches!(message, Message::Pointer(_, false)) {
            self.balk = None;
        }
        // The modes' limits, for whatever asks: a click, a key, a script,
        // the phone.
        let task = match self.refused(&message) {
            Some(why) => {
                log(format!("mode: {why}"));
                self.motion.refuse(why);
                Task::none()
            }
            None => self.handle(message),
        };
        // Whatever put it there: something new on the projector rings.
        self.motion
            .stage_shows(self.stage.program.as_ref().map(|scene| scene.id));
        self.sync_lock();
        self.motion.mode_is(self.mode);
        // The gap opens where what is held would land, as that moves.
        match (&self.drag, &self.carried) {
            // Just picked up: its gap is where it was, open already.
            (Some(drag), _) if drag.moving && self.motion.no_gap() => {
                match drag.landing == Some(drag.home) {
                    true => self.motion.lands_open(drag.home),
                    false => self.motion.lands_at(drag.landing),
                }
            }
            (Some(drag), _) if drag.moving => self.motion.lands_at(drag.landing),
            (_, Some(held)) if held.moving => {
                let landing = self.carry_landing(&self.engine.0.plan());
                self.motion.lands_at(landing);
            }
            _ => self.motion.nothing_held(),
        }
        if self.mode == mode::Mode::Plan {
            self.plans.relist();
        }
        self.sync_parts();
        self.sync_changed();
        self.sync_shelf();
        self.keep_running();
        self.sync_look();
        self.sync_blur();
        self.sync_remote();
        self.tell_remote();
        self.sync_desktop();
        Task::batch([task, self.sync_stage()])
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
        // The look of the step in hand: the one on the projector, or
        // while nothing is, the one picked in the plan. Its sections and
        // the step itself each change what the plan gives.
        let plan = self.engine.0.plan();
        let on_stage = self
            .engine
            .0
            .current_step()
            .filter(|_| self.stage.program.is_some())
            .map(|step| step.id);
        let look = match on_stage.or(self.selected).and_then(|id| plan.position(id)) {
            Some(at) => plan.look_at(at, &self.config.stage),
            None => plan
                .look
                .clone()
                .unwrap_or_else(|| self.config.stage.clone()),
        };
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
                let again = self.media.loaded(&loaded, &self.stage, &self.engine.0);
                // The next of the plan's pictures to keep loaded.
                return Task::batch([again, self.media.keep_more()]).map(Message::Loaded);
            }
            Message::Ended(scene) => {
                self.media.ended(scene, &self.engine.0);
                // A step that goes on by itself has moved the plan.
                return Task::batch([self.preload(), self.arrive()]);
            }
            Message::Remote(command) => return self.remote_command(command),
            Message::Snap => {
                // With the stage closed there is no picture to send.
                let Some(stage) = self.stage_window else {
                    return Task::none();
                };
                return window::screenshot(stage).then(|shot| {
                    Task::perform(
                        in_thread(move || {
                            phone::picture(&shot.rgba, shot.size.width, shot.size.height)
                                .map(Arc::new)
                        }),
                        Message::Snapped,
                    )
                });
            }
            Message::Snapped(Ok(jpeg)) => {
                if let Some(remote) = &self.remote {
                    remote.set_picture(jpeg.to_vec());
                }
            }
            Message::Snapped(Err(err)) => log(format!("remote: no picture: {err}")),
            Message::Broke(scene, why) => {
                self.media.broke(scene, &why, &self.engine.0);
            }
            Message::Tick => {
                if let Some((scene, position)) = self.media.playhead() {
                    self.engine.0.report(scene, MediaReport::Position(position));
                }
                // A sound has no picture to say it ended or broke through.
                match self.media.sound_event() {
                    Some((scene, Ok(()))) => return self.update(Message::Ended(scene)),
                    Some((scene, Err(why))) => self.media.broke(scene, &why, &self.engine.0),
                    None => {}
                }
            }
            Message::Progress => return self.go_on(),
            Message::Frame => {}
            Message::Acted => return self.preload(),
            Message::ScriptDone => {
                log("script finished, exiting");
                return self.leave();
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
                    Ui::Mode(to) => match (to.as_str(), mode::Mode::parse(&to)) {
                        // Starting asks twice when the look round has
                        // something to say.
                        ("start", _) => {
                            let started = self.update(Message::StartService);
                            match self.starting {
                                Some(_) => self.update(Message::StartService),
                                None => started,
                            }
                        }
                        // Ending asks twice.
                        ("end", _) => {
                            let _ = self.update(Message::EndService);
                            self.update(Message::EndService)
                        }
                        (_, Some(to)) => self.update(Message::SetMode(to)),
                        _ => Task::none(),
                    },
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
                // Each search hears only the ticket it is waiting on.
                self.search.live_event(ticket, &event, &self.engine.0);
                self.phone_search.live_event(ticket, &event, &self.engine.0);
            }
            Message::SearchDone(ticket) => {
                self.search.live_done(ticket);
                self.phone_search.live_done(ticket);
            }
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
                if window.is_some() && window == self.stage_window => {}
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
                // Pulling at a row the run won't let move is refused where
                // it can be seen, once.
                if let Some((from, pulled)) = &mut self.balk {
                    let from = *from.get_or_insert(at);
                    if !*pulled && from.distance(at) > 6.0 {
                        *pulled = true;
                        self.motion.refuse(match self.unlocked {
                            Some(_) => "only what is still ahead can change",
                            None => "the plan is locked while it runs",
                        });
                    }
                }
                if let Some(carried) = &mut self.carried {
                    carried.to(at);
                }
                if let Some(drag) = &mut self.drag {
                    drag.to(at);
                }
                self.aim();
            }
            // Pressed on a file of the library: it may be carried into
            // the plan.
            Message::Pointer(_, true) if self.shelf_hover.is_some() && self.can_edit() => {
                self.carried = self
                    .shelf_hover
                    .as_ref()
                    .and_then(|id| self.shelf_file(id))
                    .map(shelf::Carried::new);
            }
            // Let go over the plan: it goes in where the gap showed.
            Message::Pointer(_, false) if self.carried.is_some() => {
                // Where it would land, asked while it is still held.
                let plan = self.engine.0.plan();
                let landing = self.carry_landing(&plan);
                let carried = self.carried.take().filter(|carried| carried.moving);
                if let Some((carried, at)) = carried.zip(landing) {
                    self.remember(None);
                    let id = self.engine.0.edit_plan(|plan| {
                        plan.insert(
                            at,
                            carried.file.title.clone(),
                            Action::Show {
                                resource: (*carried.file).clone(),
                            },
                        )
                    });
                    self.selected = Some(id);
                    self.shelf_pick = None;
                    self.engine.0.preview(carried.file);
                    return self.preload();
                }
            }
            Message::Pointer(_, true) => {
                let plan = self.engine.0.plan();
                // Only a step the mode lets change is picked up: in a run,
                // one still ahead, once the plan is unlocked.
                let running = self.mode == mode::Mode::Run;
                self.drag = self
                    .hovered
                    .filter(|(step, _)| self.can_edit() && (!running || self.ahead(&plan, *step)))
                    .map(|(step, _)| Drag::new(&plan, step));
                if running && self.drag.is_none() && self.hovered.is_some() {
                    self.balk = Some((None, false));
                }
            }
            Message::Pointer(_, false) => {
                let dropped = self.drag.take().filter(|drag| drag.moving);
                // In a run it lands after the step the service is at, or
                // not at all.
                let reached = (self.mode == mode::Mode::Run)
                    .then(|| self.engine.0.current_step())
                    .flatten()
                    .and_then(|step| self.engine.0.plan().position(step.id));
                let dropped = dropped.filter(|drag| {
                    drag.landing
                        .is_none_or(|at| reached.is_none_or(|reached| at > reached))
                });
                // Let go where it was picked up, nothing moved.
                let dropped = dropped
                    .and_then(|d| d.landing.map(|at| (d, at)))
                    .filter(|(drag, at)| *at != drag.home);
                if let Some((drag, at)) = dropped {
                    self.remember(None);
                    self.engine
                        .0
                        .edit_plan(|plan| plan.move_steps(&drag.carried, at));
                    if let Some(first) = drag.carried.first() {
                        self.motion.changed(*first);
                    }
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
                    // Looked at on this screen; the plan's selection
                    // stays, so it is added after the selected step.
                    Clicked::Shelf(id) => {
                        if let Some(file) = self.shelf_file(&id) {
                            self.search.unpick();
                            self.shelf_pick = Some(id);
                            self.shelf_folder = None;
                            self.in_library = true;
                            self.engine.0.preview(file);
                        }
                        self.preload()
                    }
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
            Message::KeyIfClosed { .. } if self.settings.is_some() => {}
            Message::KeyIfClosed { typed, .. } if self.search_open => {
                return self.update(Message::Type(typed));
            }
            // Someone typing a search into a closed box hasn't asked for
            // the next step: the Space between their words is let go.
            Message::KeyIfClosed { op, .. } => {
                if self.stray.is_some_and(|at| at.elapsed() < STRAY) {
                    self.stray = Some(Instant::now());
                } else {
                    return self.op(op);
                }
            }
            // With the search closed there's nowhere to type.
            Message::Type(_) if !self.search_open => self.stray = Some(Instant::now()),
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
            Message::Attachment(n) => {
                let Some(scene) = self.stage.program.clone() else {
                    return Task::none();
                };
                let Some(resource) = self
                    .attached
                    .as_ref()
                    .and_then(|(_, slides)| slides.get(scene.frame())?.get(n).cloned())
                else {
                    return Task::none();
                };
                // A video inside the deck came out when the deck loaded.
                // If it didn't, it does now, or the stage says why not.
                if let (Some(entry), lantern_engine::Location::File(deck)) =
                    (resource.meta.get(deck::ENTRY), &scene.resource.location)
                    && let Err(err) = deck::take_out(deck, entry, &stage::cache_dir().join("decks"))
                {
                    log(format!("deck: {err}"));
                }
                self.left = Some(Left {
                    deck: Arc::clone(&scene.resource),
                    frame: scene.frame(),
                    shown: resource.id.clone(),
                    more: scene.has_next_frame(),
                });
                self.engine.0.show_now(resource);
                self.stepping = true;
            }
            Message::BackToSlides { next } => {
                if let Some(left) = self.left.take() {
                    self.engine
                        .0
                        .show_now_at(left.deck, left.frame + usize::from(next));
                    self.stepping = true;
                }
            }
            Message::ShelfFold(folder) => {
                if !self.shelf_open.remove(&folder) {
                    self.shelf_open.insert(folder.clone());
                }
                // The arrows go on from where the click was.
                self.shelf_folder = Some(folder);
                self.shelf_pick = None;
                self.in_library = true;
            }
            Message::ShelfHover(file) => self.shelf_hover = file,
            Message::ShelfFilter(typed) => {
                self.shelf_filter = typed;
            }
            Message::ReplaceStep(id) => {
                let Some((step, pick)) = self.replaceable_at(id) else {
                    return Task::none();
                };
                self.remember(None);
                self.engine.0.edit_plan(|plan| {
                    let Some(step) = plan.get_mut(id) else {
                        return;
                    };
                    // A step named after what it showed is named after
                    // what it shows now; a name the operator gave stays.
                    let named_after = matches!(
                        &step.action,
                        Action::Show { resource } if resource.title == step.label
                    );
                    if named_after || step.label.is_empty() {
                        step.label.clone_from(&pick.title);
                    }
                    // The sound that played with it stays with the step;
                    // a length set for the old content doesn't fit the new.
                    step.estimate = None;
                    step.action = Action::Show {
                        resource: (*pick).clone(),
                    };
                });
                log(format!("plan: {} replaced with {}", step.label, pick.title));
                self.search.unpick();
                self.shelf_pick = None;
                self.engine.0.preview(pick);
                return self.preload();
            }
            Message::StepSlide(id, slide) => {
                let Some(step) = self.engine.0.plan().get(id).cloned() else {
                    return Task::none();
                };
                let Action::Show { resource } = &step.action else {
                    return Task::none();
                };
                let on_stage = self
                    .stage
                    .picture()
                    .is_some_and(|scene| scene.resource.id == resource.id)
                    && self.engine.0.current_step().is_some_and(|at| at.id == id);
                match self.mode {
                    // The step is on the projector: its slide goes on.
                    mode::Mode::Run if on_stage => {
                        self.engine.0.goto_frame(slide);
                        self.stepping = true;
                    }
                    // Looked at, at that slide, on this screen.
                    _ => {
                        self.selected = Some(id);
                        self.shelf_pick = None;
                        self.search.unpick();
                        self.engine.0.preview_at(Arc::new(resource.clone()), slide);
                    }
                }
                return self.preload();
            }
            Message::Fold(section) => {
                let closed = !self.folded.remove(&section);
                if closed {
                    self.folded.insert(section);
                }
                self.motion.folded(section, closed);
            }
            Message::SetMode(to) => return self.set_mode(to),
            Message::StartService => return self.start_service(),
            Message::EndService => return self.end_service(),
            Message::NotYet => self.starting = None,
            Message::SaveSession => return self.save_session(),
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
                if self.from_typed.as_ref().is_some_and(|(id, _)| *id != step) {
                    self.from_typed = None;
                }
                self.selected = Some(step);
                self.shelf_pick = None;
                self.shelf_folder = None;
                self.in_library = false;
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
            Message::KeepCopy(id) => {
                let plan = self.engine.0.plan();
                let Some(file) = plan
                    .get(id)
                    .and_then(|step| self.loose(step))
                    .map(Path::to_path_buf)
                else {
                    return Task::none();
                };
                let dir = self
                    .plans
                    .store()
                    .bundles()
                    .join(bundle::file_safe(&plan.title));
                self.copying = true;
                self.plans.notice = Some(plans::Notice::Done("Copying…".into()));
                return Task::perform(
                    in_thread(move || {
                        bundle::keep(&file, &dir)
                            .map_err(|err| format!("Couldn't copy {}: {err}", file.display()))
                    }),
                    move |kept| Message::KeptCopy(id, kept),
                );
            }
            Message::KeptCopy(id, kept) => {
                self.copying = false;
                match kept {
                    Ok(copy) => {
                        self.remember(None);
                        let title = self.engine.0.edit_plan(|plan| {
                            let step = plan.get_mut(id)?;
                            let Action::Show { resource } = &mut step.action else {
                                return None;
                            };
                            // The same thing, at the copy: the step keeps
                            // its name, its length and its sound.
                            resource.id = ResourceId::new(
                                &lantern_engine::SourceId::new("link").expect("a valid source id"),
                                &copy.to_string_lossy(),
                            );
                            resource.location = lantern_engine::Location::File(copy.clone());
                            resource.subtitle = Some("Kept with the plan".into());
                            Some(resource.title.clone())
                        });
                        if let Some(title) = title {
                            log(format!(
                                "plan: kept a copy of {title} at {}",
                                copy.display()
                            ));
                            self.plans.notice = Some(plans::Notice::Done(format!(
                                "“{title}” is kept with the plan now"
                            )));
                        }
                        return self.preload();
                    }
                    Err(why) => self.plans.notice = Some(plans::Notice::Failed(why)),
                }
            }
            Message::StepSound(id, sound) => {
                self.remember(None);
                self.engine.0.edit_plan(|plan| {
                    if let Some(step) = plan.get_mut(id) {
                        self.from_typed = None;
                        step.sound = sound.map(|resource| lantern_engine::StepSound {
                            resource: (*resource).clone(),
                            from: 0,
                        });
                    }
                })
            }
            Message::SoundFrom(id, typed) => {
                // A slide's number: digits, a few. Anything else isn't
                // typed. Empty stands for the first slide while it lasts.
                let typed = typed.trim().to_string();
                if typed.len() > 3 || !typed.bytes().all(|b| b.is_ascii_digit()) {
                    return Task::none();
                }
                let from = typed.parse::<usize>().unwrap_or(1).max(1);
                self.remember(Some(id));
                self.engine.0.edit_plan(|plan| {
                    if let Some(sound) = plan.get_mut(id).and_then(|s| s.sound.as_mut()) {
                        sound.from = from - 1;
                    }
                });
                self.from_typed = Some((id, typed));
            }
            Message::Rehearse(of) => return self.rehearse(of),
            Message::RehearseNext => return self.rehearse_next(),
            Message::RehearsePause => self.rehearse_pause(),
            Message::RehearseSpeed => self.rehearse_speed(),
            Message::RehearseTick => return self.rehearse_tick(),
            Message::Lens(lens) => {
                self.lens = lens;
                self.look_typed = None;
                // The arrows are the library's only while it is in sight.
                if lens != side::Lens::Add {
                    self.in_library = false;
                }
                if lens == side::Lens::Check {
                    self.sound_now = before::sound();
                }
            }
            Message::LookEdit(field, typed) => self.look_edit(field, typed),
            Message::LookReset(field) => self.look_reset(field),
            Message::Unselect => {
                self.selected = None;
                self.look_typed = None;
                self.engine.0.clear_preview();
            }
            Message::EditTarget => {
                let target = self.engine.0.plan().target;
                self.target_typed = Some(target.map(length::typed).unwrap_or_default());
                return operation::focus(plan_pane::TARGET_BOX);
            }
            Message::PlanTarget(text) => {
                // Kept as typed until it reads as a length. Emptied, the
                // plan has no target.
                if length::typeable(&text) {
                    if let Some(target) = length::parse(&text) {
                        self.engine.0.edit_plan(|plan| plan.target = target);
                    }
                    self.target_typed = Some(text);
                }
            }
            Message::TargetDone => self.target_typed = None,
            Message::PlanAdvance(on) => {
                self.remember(None);
                self.engine.0.edit_plan(|plan| plan.advance = on);
            }
            Message::SetRepeat(of, repeat) => {
                self.remember(None);
                self.engine.0.edit_plan(|plan| match of {
                    Some(id) => {
                        if let Some(step) = plan.get_mut(id) {
                            step.flow.repeat = repeat;
                        }
                    }
                    None => plan.flow.repeat = repeat,
                });
            }
            Message::SetShuffle(of, on) => {
                self.remember(None);
                self.engine.0.edit_plan(|plan| match of {
                    Some(id) => {
                        if let Some(step) = plan.get_mut(id) {
                            step.flow.shuffle = on;
                        }
                    }
                    None => plan.flow.shuffle = on,
                });
            }
            Message::RepeatPanel(of) => {
                self.repeat_panel = (self.repeat_panel != Some(of)).then_some(of);
                self.repeat_until.clear();
            }
            Message::RepeatUntil(typed) => {
                self.repeat_until = typed
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == ':')
                    .take(5)
                    .collect();
            }
            Message::RepeatUntilSet => {
                if let (Some(of), Some(time)) = (
                    self.repeat_panel,
                    lantern_engine::TimeOfDay::parse(&self.repeat_until),
                ) {
                    self.repeat_until.clear();
                    return self
                        .update(Message::SetRepeat(of, lantern_engine::Repeat::Until(time)));
                }
            }
            Message::SetAdvance(id, on) => {
                self.remember(None);
                self.engine.0.edit_plan(|plan| {
                    if let Some(step) = plan.get_mut(id) {
                        step.advance = Some(on);
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
                // In a run, never up past the step the service is at.
                let reached = (self.mode == mode::Mode::Run)
                    .then(|| self.engine.0.current_step())
                    .flatten();
                self.remember(None);
                self.engine.0.edit_plan(|plan| {
                    let floor = reached
                        .and_then(|step| plan.position(step.id))
                        .map_or(0, |reached| reached + 1);
                    if let Some(at) = plan.position(id) {
                        plan.move_to(id, at.saturating_add_signed(by).max(floor));
                    }
                });
                return self.preload();
            }
            Message::Nest(id, inward) => {
                // Asked before the edit is kept for undo: one that moves
                // nothing leaves nothing to take back.
                let mut tried = self.engine.0.plan();
                if tried.get(id).is_some_and(|step| !step.is_section()) {
                    self.motion.refuse("only a section goes inside another");
                } else if !tried.nest(id, inward) {
                    self.motion.refuse(match inward {
                        true => "it can't go further in",
                        false => "it is as far out as it goes",
                    });
                } else {
                    self.remember(None);
                    self.engine.0.edit_plan(|plan| {
                        plan.nest(id, inward);
                    });
                }
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
            // Closing the dashboard ends Lantern, and with it whatever is
            // on the projector: with something there, it is asked twice.
            Message::CloseAsked(window) if window == self.dashboard_window => {
                let showing = match self.mode {
                    mode::Mode::Run => true,
                    mode::Mode::Live => self.stage.program.is_some(),
                    mode::Mode::Plan => false,
                };
                let sure = self
                    .quitting
                    .is_some_and(|asked| asked.elapsed() < SURE_QUIT);
                if showing && !sure {
                    self.quitting = Some(Instant::now());
                    self.motion.refuse("close again to quit");
                    log("the dashboard was asked to close mid-service: asking again");
                } else {
                    log("the dashboard closed, exiting");
                    return self.leave();
                }
            }
            Message::CloseAsked(window) => return window::close(window),
            // The dashboard closing ends Lantern. The stage closing ends
            // only the stage: closed from outside, it stays closed until
            // it is opened again, like one closed by hand.
            Message::Closed(window) if window == self.dashboard_window => {
                log("the dashboard closed, exiting");
                return self.leave();
            }
            Message::Closed(window) => {
                if Some(window) == self.stage_window {
                    log("stage: the window was closed");
                    self.stage_window = None;
                    self.stage_wish = Some(false);
                }
            }
            Message::Resized(window, size) if Some(window) != self.stage_window => {
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
        // In a rehearsal, Space moves it on as it would the service.
        if op == Op::Next && self.rehearsal.is_some() {
            return self.rehearse_next();
        }
        let op = match self.op_in_mode(op) {
            Ok(op) => op,
            Err(why) => {
                log(format!("mode: {why}"));
                self.motion.refuse(why);
                return Task::none();
            }
        };
        match op {
            // Behind the settings nothing of the switch is in sight.
            Op::Mode(_) if self.settings.is_some() => {}
            Op::Mode(to) => {
                return match (self.mode, to) {
                    (mode::Mode::Plan, mode::Mode::Run) => self.start_service(),
                    (mode::Mode::Run, mode::Mode::Plan) => self.end_service(),
                    (_, to) => self.set_mode(to),
                };
            }
            Op::Unlock => self.toggle_unlock(),
            // By hand: it holds until the mode changes.
            Op::Stage => self.stage_wish = Some(self.stage_window.is_none()),
            Op::Pane => return self.switch_pane(),
            // The library has the arrows: they walk its tree.
            Op::PickNext | Op::PickPrev | Op::PickFirst | Op::PickLast
                if !self.searching() && self.keys_in_library() =>
            {
                return self.shelf_move(go(op));
            }
            Op::In | Op::Out if !self.searching() && self.keys_in_library() => {
                return self.shelf_side(op == Op::In);
            }
            Op::Enter if !self.searching() && self.keys_in_library() && self.shelf_enter() => {}
            // In the plan, → opens a section and then steps into it; ←
            // closes it, and from a step goes up to its heading.
            Op::In | Op::Out => {
                let plan = engine.plan();
                let steps = plan.steps();
                let in_plan = !matches!(self.plans.mode, plans::Mode::Browsing(_));
                let at = self
                    .selected
                    .filter(|_| in_plan && !self.searching())
                    .and_then(|id| plan.position(id));
                let Some(at) = at else {
                    return Task::none();
                };
                let into = op == Op::In;
                let step = &steps[at];
                if step.is_section() {
                    let folded = self.folded.contains(&step.id);
                    // Into it: its first row, a step or a section in it.
                    let inside = steps.get(at + 1).filter(|_| plan.group_end(at) > at + 1);
                    match (into, folded, inside) {
                        (true, true, _) | (false, false, _) => {
                            return self.update(Message::Fold(step.id));
                        }
                        (true, false, Some(inside)) => {
                            return self.update(Message::Select(inside.id));
                        }
                        _ => {}
                    }
                } else if !into && let Some(above) = plan.within(at).last() {
                    return self.update(Message::Select(*above));
                }
            }
            // A service cut short is picked up where it was: its step
            // goes on, at its slide.
            Op::Next if self.resuming.is_some() => {
                if let Some((step, frame)) = self.resuming.take()
                    && let Err(err) = engine.go_to_at(step, frame)
                {
                    log(format!("mode: can't pick the service up: {err}"));
                }
                self.stepping = true;
                return Task::batch([self.preload(), self.arrive()]);
            }
            // In Live there is no plan to step through: Next and Prev
            // turn the slides of what is on stage, and no more.
            Op::Next | Op::Prev if self.mode == mode::Mode::Live && !self.left_for_program() => {
                let at = self.stage.picture().map(lantern_engine::Scene::frame);
                let to = match (at, op == Op::Next) {
                    (Some(at), true) => Some(at + 1),
                    (Some(at), false) => at.checked_sub(1),
                    (None, _) => None,
                };
                if let Some(to) = to {
                    engine.goto_frame(to);
                    self.stepping = true;
                }
            }
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
            // From what a slide played, Next and Prev go back to the
            // slides: on to the one after, or back to the one it was on.
            Op::Next | Op::Prev if self.left_for_program() => {
                return self.update(Message::BackToSlides {
                    next: op == Op::Next,
                });
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
            Op::PickNext | Op::PickPrev | Op::PickFirst | Op::PickLast if !self.searching() => {
                let go = go(op);
                let around = |at: Option<usize>, len: usize| go.from(at, len);
                // Home and End bring their end of the pane into sight.
                let ends = match go {
                    shelf::Go::First => {
                        operation::snap_to(plan_pane::ROWS, scrollable::RelativeOffset::START)
                    }
                    shelf::Go::Last => {
                        operation::snap_to(plan_pane::ROWS, scrollable::RelativeOffset::END)
                    }
                    _ => Task::none(),
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
                    // The steps in sight, then what's on standby under
                    // them. A folded section's steps are passed over.
                    let plan = engine.plan();
                    let current = engine.current_step().map(|step| step.id);
                    let mut hidden_to = 0;
                    let seen: Vec<StepId> = plan
                        .steps()
                        .iter()
                        .enumerate()
                        .filter(|(index, step)| {
                            let hidden = *index < hidden_to;
                            if step.is_section() {
                                if !hidden && self.folded.contains(&step.id) {
                                    hidden_to = plan.group_end(*index);
                                }
                                return !hidden;
                            }
                            !hidden || Some(step.id) == current || Some(step.id) == self.selected
                        })
                        .map(|(_, step)| step.id)
                        .collect();
                    let steps = seen.len();
                    let at = match self
                        .selected
                        .and_then(|id| seen.iter().position(|seen| *seen == id))
                    {
                        Some(at) => Some(at),
                        None => self.standby_lined_up().map(|at| steps + at),
                    };
                    match around(at, steps + plan.standby.len()) {
                        Some(at) if at < steps => {
                            return Task::batch([self.update(Message::Select(seen[at])), ends]);
                        }
                        Some(at) => {
                            return Task::batch([
                                self.line_up_standby(&plan.standby[at - steps].id),
                                ends,
                            ]);
                        }
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
            // In a search Home and End are the box's own.
            Op::PickFirst | Op::PickLast => {}
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
                // Esc empties the search and keeps its bang, for the next
                // search of the same source; on an empty box it closes
                // the search, and the bang goes with it.
                let closing = !self.search_open || self.search.query.is_empty();
                if closing {
                    self.search_open = false;
                }
                self.adding = None;
                self.editing = None;
                self.list_pick = None;
                self.importing = None;
                if closing {
                    self.search.clear(&engine);
                } else {
                    self.search.empty(&engine);
                }
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
                    self.redo.push(engine.plan());
                    return self.put_back(before);
                }
            }
            Op::Redo => {
                if let Some(after) = self.redo.pop() {
                    self.undo.push(engine.plan());
                    return self.put_back(after);
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
            Op::NestIn | Op::NestOut => {
                if let Some(step) = self.selected {
                    return self.update(Message::Nest(step, op == Op::NestIn));
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
    /// Looks at the screens: decides where the stage goes with those
    /// connected, remembers a stage moved by hand, and moves the stage to
    /// its screen when it first appears, when its screen changes, or when
    /// that screen is plugged in.
    fn check_outputs(&mut self) {
        let (Ok(connected), Ok(stage)) = (outputs::list(), outputs::stage_on()) else {
            return;
        };
        let setup = outputs::setup(&connected);
        if setup != self.setup {
            log(format!("outputs: connected: {setup}"));
            // Other screens: what was done by hand was for the old ones.
            self.stage_wish = None;
            self.stage_seen_on = None;
            self.setup = setup;
        }
        // Seen on one screen and now on another, with nothing here having
        // moved it: the operator did, and that is where it goes from now
        // on with these screens.
        if let (Some(before), Some(now)) = (&self.stage_seen_on, &stage)
            && before != now
            && let Some(screen) = connected.iter().find(|output| &output.name == now)
        {
            log(format!("outputs: the stage was moved to {now}: remembered"));
            if let Err(err) = self.screens.keep(&self.setup, screen.id()) {
                log(format!("outputs: can't remember it: {err}"));
            }
        }
        self.stage_seen_on = stage.clone();
        let remembered = self.screens.of(&self.setup);
        let dashboard = outputs::dashboard_on().ok().flatten();
        let place = outputs::choose(
            &connected,
            self.config.stage_output.as_deref(),
            remembered.as_deref(),
            dashboard.as_deref(),
        );
        if self.place.as_ref() != Some(&place) {
            log(format!("outputs: the stage goes {place:?}"));
        }
        let wanted = place.output().map(str::to_string);
        self.place = Some(place);
        let names = connected.into_iter().map(|o| o.name);
        if self
            .output_watch
            .check(names, wanted.as_deref(), stage.is_some())
            && let Some(output) = wanted.as_deref()
            && stage.as_deref() != Some(output)
        {
            move_stage(output);
            // Moved from here: the next look is not a move by hand.
            self.stage_seen_on = None;
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
                self.motion.still = self.config.no_motion;
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
        // A new edit: what was undone before it can't follow it.
        self.redo.clear();
    }

    /// Makes the plan `to`, for an undo or a redo: nothing is mid-edit
    /// any more, and a step that is gone isn't selected.
    fn put_back(&mut self, to: Plan) -> Task<Message> {
        self.editing = None;
        self.typing = None;
        self.drafts.clear();
        self.from_typed = None;
        self.engine.0.edit_plan(|plan| *plan = to);
        let plan = self.engine.0.plan();
        self.selected = self.selected.filter(|id| plan.get(*id).is_some());
        self.preload()
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
        // Another plan's sections are folded or not on their own account.
        if replaced {
            self.folded.clear();
        }
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
            self.redo.clear();
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
        for scene in [&self.stage.program, &self.stage.preview, &self.stage.under]
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

    /// Reads what each deck in the plan plays or links to, for the plan
    /// pane to show inside its step. Once a deck: opening it is a file
    /// read, not something to do each time the pane is drawn.
    /// Flashes the step that was just added or that shows something else
    /// now, however it came about: a key, a drop, the Replace button. One
    /// step, in the plan that was open already: opening another plan or
    /// taking back several edits flashes nothing.
    fn sync_changed(&mut self) {
        let file = self.plans.file().map(Path::to_path_buf);
        let steps: HashMap<StepId, Option<ResourceId>> = self.engine.0.edit_plan(|plan| {
            plan.steps()
                .iter()
                .map(|step| {
                    let shows = match &step.action {
                        Action::Show { resource } => Some(resource.id.clone()),
                        _ => None,
                    };
                    (step.id, shows)
                })
                .collect()
        });
        if steps == self.seen.1 {
            self.seen.0 = file;
            return;
        }
        let same_plan = file == self.seen.0
            && (steps.len() == 1 || steps.keys().any(|id| self.seen.1.contains_key(id)));
        let mut changed = steps
            .iter()
            .filter(|(id, shows)| self.seen.1.get(*id) != Some(*shows))
            .map(|(id, _)| *id);
        if let (true, Some(step), None) = (same_plan, changed.next(), changed.next()) {
            self.motion.changed(step);
        }
        self.seen = (file, steps);
    }

    /// The file `step` shows, when it is one that was named by its path
    /// and is in none of the media folders: a download, a file on a USB
    /// stick. It may move or go, and the plan with it, until a copy is
    /// kept.
    pub(crate) fn loose<'a>(&self, step: &'a Step) -> Option<&'a Path> {
        let Action::Show { resource } = &step.action else {
            return None;
        };
        let lantern_engine::Location::File(path) = &resource.location else {
            return None;
        };
        (resource.id.source() == "link" && !path.starts_with(self.plans.store().bundles()))
            .then_some(path.as_path())
    }

    fn sync_parts(&mut self) {
        let decks: Vec<lantern_engine::Resource> = self.engine.0.edit_plan(|plan| {
            plan.steps()
                .iter()
                .filter_map(|step| match &step.action {
                    Action::Show { resource }
                        if resource.kind == lantern_engine::Kind::Presentation
                            && !self.parts.contains_key(&resource.id) =>
                    {
                        Some(resource.clone())
                    }
                    _ => None,
                })
                .collect()
        });
        for deck in decks {
            let parts = deck::attached(&deck, &stage::cache_dir().join("decks"));
            self.parts.insert(deck.id.clone(), parts);
        }
    }

    /// Where in `plan` a file carried from the library would land: above
    /// or below the step the pointer is over. Nowhere while it isn't over
    /// one, or isn't being carried.
    pub(crate) fn carry_landing(&self, plan: &Plan) -> Option<usize> {
        self.carried.as_ref().filter(|carried| carried.moving)?;
        let (over, half) = self.hovered?;
        let at = plan.position(over)? + usize::from(half == Half::Lower);
        // In a run, only after the step the service is at.
        let reached = (self.mode == mode::Mode::Run)
            .then(|| self.engine.0.current_step())
            .flatten()
            .and_then(|step| plan.position(step.id));
        reached.is_none_or(|reached| at > reached).then_some(at)
    }

    /// Writes down where the running service is, when it has moved, and
    /// forgets it once the service has ended. A service cut short by a
    /// crash or a restart is found here at the next start.
    fn keep_running(&mut self) {
        if self.mode != mode::Mode::Run {
            if self.kept.take().is_some() {
                let _ = self.plans.store().set_running(None);
            }
            return;
        }
        // Reached a step: whatever was waiting to be picked up is behind.
        let Some(step) = self.engine.0.current_step() else {
            return;
        };
        self.resuming = None;
        let Some(plan) = self.plans.file().map(std::path::Path::to_path_buf) else {
            return;
        };
        let frame = match &step.action {
            Action::Show { resource } => self
                .stage
                .picture()
                .filter(|scene| scene.resource.id == resource.id)
                .map_or(0, lantern_engine::Scene::frame),
            _ => 0,
        };
        let now = plans::Running {
            plan,
            step: step.id,
            frame,
        };
        if self.kept.as_ref() != Some(&now) {
            if let Err(err) = self.plans.store().set_running(Some(&now)) {
                log(format!("mode: can't keep the service's place: {err}"));
            }
            self.kept = Some(now);
        }
    }

    /// At startup: if a service was running when Lantern last stopped, and
    /// its plan is the one open, Lantern comes back in Run with the plan
    /// waiting at that step. Nothing is shown until the operator says so.
    fn find_running(&mut self) {
        let Some(running) = self.plans.store().running() else {
            return;
        };
        let plan = self.engine.0.plan();
        if self.plans.file() == Some(running.plan.as_path()) && plan.get(running.step).is_some() {
            self.mode = mode::Mode::Run;
            self.resuming = Some((running.step, running.frame));
            self.selected = Some(running.step);
            log(format!(
                "mode: a service was running, at step {}",
                running.step
            ));
            self.kept = Some(running);
        } else {
            let _ = self.plans.store().set_running(None);
        }
    }

    /// Whether what is on stage is something a slide played or linked to,
    /// with its deck to go back to.
    pub(crate) fn left_for_program(&self) -> bool {
        match (&self.left, &self.stage.program) {
            (Some(left), Some(scene)) => scene.resource.id == left.shown,
            _ => false,
        }
    }

    /// Keeps up with a deck on stage: reads what its slides play or link
    /// to when it goes on, and forgets the deck that was left once the
    /// stage has gone on to something else.
    fn follow_deck(&mut self) {
        let program = self.stage.program.as_ref().map(|scene| &scene.resource);
        if let Some(left) = &self.left
            && program.is_none_or(|on| on.id != left.shown && on.id != left.deck.id)
        {
            self.left = None;
        }
        let deck = program.filter(|on| {
            on.kind == lantern_engine::Kind::Presentation
                && matches!(&on.location, lantern_engine::Location::File(path) if deck::is_deck(path))
        });
        match deck {
            Some(on) => {
                if self.attached.as_ref().is_none_or(|(id, _)| *id != on.id) {
                    let slides = deck::attached(on, &stage::cache_dir().join("decks"));
                    self.attached = Some((on.id.clone(), slides));
                }
            }
            None => self.attached = None,
        }
    }

    fn program_changed(&mut self) -> Task<Message> {
        self.follow_deck();
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
        let step = self
            .engine
            .0
            .current_step()
            .filter(|_| self.engine.0.goes_on())?;
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
    /// The text of `scene` now: its current slide, or for a clock, timer
    /// or countdown what it reads this second. A countdown counts from
    /// when it went on stage; lined up, it shows where it would start.
    pub(crate) fn scene_text(&self, scene: &lantern_engine::Scene) -> Option<String> {
        let Some(clock) = lantern_source_clock::Clock::of(&scene.resource) else {
            return self.slides.slide(scene);
        };
        let on_stage = match (&self.stage.program, self.shown_since) {
            (Some(program), Some((Some(shown), _, since)))
                if program.id == scene.id && shown == scene.id =>
            {
                Some(since.elapsed())
            }
            _ => None,
        };
        Some(clock.text(on_stage, length::now_of_day()))
    }

    /// How `resource`'s slide `frame` is set: a clock large, a passage
    /// with its verse in the corner (unless the look leaves it out).
    pub(crate) fn slide_kind(&self, resource: &lantern_engine::Resource, frame: usize) -> Slide {
        if lantern_source_clock::Clock::of(resource).is_some() {
            return Slide::Clock;
        }
        let passage = resource.meta.get("provider").map(String::as_str) == Some("bible");
        if !passage || self.look.no_reference {
            return Slide::Plain;
        }
        let verses: Vec<String> = resource
            .meta
            .get("verses")
            .map(|list| list.split(',').map(|v| v.trim().to_string()).collect())
            .unwrap_or_default();
        let labels = match resource.location.as_text() {
            Some(body) => self.slides.labels(body, &verses),
            None => Vec::new(),
        };
        Slide::Passage(reference(
            &resource.title,
            labels.get(frame).map(String::as_str),
            resource.meta.get("translation").map(String::as_str),
        ))
    }

    /// Whether a clock, timer or countdown is on stage or lined up, so
    /// the windows redraw as it ticks.
    fn ticking(&self) -> bool {
        [&self.stage.program, &self.stage.preview]
            .into_iter()
            .flatten()
            .any(|scene| lantern_source_clock::Clock::of(&scene.resource).is_some())
    }

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
        if Some(window) == self.stage_window {
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
        // Under a sound, the picture it was started over.
        let Some(scene) = self.stage.picture() else {
            // Blackout is black even with nothing on: not the look's image.
            if self.stage.blackout {
                return black(Space::new());
            }
            return backdrop(look, Space::new(), Over::Nothing);
        };
        let kind = self.slide_kind(&scene.resource, scene.frame());
        let nothing = || (Space::new().into(), Over::Nothing);
        let (content, over): (Element<'_, Message>, Over) =
            match (&scene.media, self.scene_text(scene)) {
                (Media::Failed { .. }, _) => nothing(),
                (_, Some(body)) => (slide(look, body, &kind), Over::Text),
                (_, None) => match self.media.content(scene.id) {
                    None => nothing(),
                    Some(Content::Image(handle)) => (fit(image(handle.clone())), Over::Media),
                    Some(Content::Pages(pages)) => {
                        let page = &pages[scene.frame().min(pages.len() - 1)];
                        (fit(image(page.clone())), Over::Media)
                    }
                    Some(Content::Sound(_)) => nothing(),
                    Some(Content::Video { player, .. }) => (
                        VideoPlayer::new(player)
                            .width(Fill)
                            .height(Fill)
                            .content_fit(ContentFit::Contain)
                            .on_end_of_stream(Message::Ended(scene.id))
                            .on_error({
                                let id = scene.id;
                                move |err| Message::Broke(id, err.message().to_string())
                            })
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
            window::close_requests().map(Message::CloseAsked),
            window::resize_events().map(|(window, size)| Message::Resized(window, size)),
            iced::event::listen_with(keys::key_event),
            iced::event::listen_with(pointer_event),
        ];
        // The pointer is followed only while it holds a row or a pane's edge.
        if self.drag.is_some()
            || self.resizing.is_some()
            || self.carried.is_some()
            || self.balk.is_some()
        {
            subscriptions.push(iced::event::listen_with(cursor_event));
        }
        // A search on screen follows files added to or removed from a folder.
        if !self.search.query.is_empty() {
            subscriptions
                .push(iced::time::every(Duration::from_secs(1)).map(|_| Message::CheckIndex));
        }
        // The stage's screen, when one is asked for: at startup, and when
        // it's plugged in later.
        if outputs::supported() {
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
        // A rehearsal's clock.
        if self.rehearsal.is_some() {
            subscriptions
                .push(iced::time::every(Duration::from_millis(200)).map(|_| Message::RehearseTick));
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
        // What shows time is drawn again several times a second, so it
        // never skips one.
        if self.ticking() {
            subscriptions
                .push(iced::time::every(Duration::from_millis(200)).map(|_| Message::Progress));
        }
        if self.downloads.0.busy() || self.library.0.indexing() || self.thumbs.busy() {
            subscriptions
                .push(iced::time::every(Duration::from_millis(250)).map(|_| Message::Progress));
        }
        // What moves is drawn every frame, and only while it moves; what
        // shows for a while is looked at often enough to come down on time.
        if self.motion.moving() {
            subscriptions.push(window::frames().map(|_| Message::Frame));
        } else if self.motion.showing() {
            subscriptions
                .push(iced::time::every(Duration::from_millis(200)).map(|_| Message::Frame));
        }
        if let Some(remote) = &self.remote {
            subscriptions.push(Subscription::run_with(
                Shared(remote.inbox.clone()),
                remote_feed,
            ));
            // What the page shows moves with the clock (a section's time,
            // how far behind the service is), also when nothing else does.
            if remote.watched() {
                subscriptions
                    .push(iced::time::every(Duration::from_secs(1)).map(|_| Message::Progress));
            }
            // A picture of the stage, while a phone is showing it.
            if remote.wants_picture() {
                subscriptions
                    .push(iced::time::every(Duration::from_millis(700)).map(|_| Message::Snap));
            }
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
        Action::Section { budget, .. } => *budget = value,
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
/// What phones send through the remote, as messages.
fn remote_feed(inbox: &Shared<phone::Inbox>) -> impl Stream<Item = Message> + use<> {
    let commands = inbox.0.take();
    iced::stream::channel(64, async move |mut out| {
        let Some(mut commands) = commands else {
            return;
        };
        while let Some(command) = commands.next().await {
            if out.send(Message::Remote(command)).await.is_err() {
                break;
            }
        }
    })
}

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
/// How a slide of text is set.
pub(crate) enum Slide {
    Plain,
    /// A clock, timer or countdown: a few figures, set large.
    Clock,
    /// A Bible passage, with this in the stage's lower corner: the verse
    /// it's at and the translation.
    Passage(String),
}

/// What the corner says of a passage: `Juan 3:17 · RVG`. `title` is the
/// passage's (`Juan 3:16-18`), `verse` the slide's (`17`, or `17a` for
/// the first part of a long one). Without a verse, or for a passage over
/// more than one chapter, the title as it is.
fn reference(title: &str, verse: Option<&str>, translation: Option<&str>) -> String {
    let verse = verse.map(|v| v.trim_end_matches(|c: char| c.is_ascii_lowercase()));
    let chapter = title.split(':').next().unwrap_or(title);
    let at = match verse.filter(|v| !v.is_empty() && title.matches(':').count() <= 1) {
        Some(verse) => format!("{chapter}:{verse}"),
        None => title.to_string(),
    };
    match translation {
        Some(translation) => format!("{at} · {translation}"),
        None => at,
    }
}

/// `body` as the stage sets it: as large as fits, and for a passage its
/// reference small in the lower right corner, inside the margin the text
/// leaves free.
pub(crate) fn slide<'a>(look: &Look, body: String, kind: &Slide) -> Element<'a, Message> {
    use iced::Length::FillPortion;
    match kind {
        Slide::Plain => fit::fit_text(body, look).into(),
        Slide::Clock => fit::fit_text(body, &clock_look(look)).into(),
        Slide::Passage(corner) => {
            // A box a twelfth of the stage tall; its text fills it.
            let small = Look {
                min_size: 240,
                max_size: 600,
                ..look.clone()
            };
            let corner = iced::widget::column![
                Space::new().height(FillPortion(11)),
                iced::widget::row![
                    Space::new().width(FillPortion(5)),
                    container(fit::fit_text(corner.clone(), &small)).width(FillPortion(4)),
                ]
                .height(FillPortion(1)),
            ];
            stack![fit::fit_text(body, look), corner].into()
        }
    }
}

/// `look` for a clock, timer or countdown: the same, but with its few
/// figures allowed several times the size a passage's text gets.
pub(crate) fn clock_look(look: &Look) -> Look {
    let (least, most) = look.sizes();
    Look {
        min_size: least,
        max_size: most.saturating_mul(4).max(360),
        ..look.clone()
    }
}

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
    use super::{reference, typed_into};

    #[test]
    fn the_corner_says_the_verse_the_passage_is_at() {
        assert_eq!(
            reference("Juan 3:16-18", Some("17"), Some("RVG")),
            "Juan 3:17 · RVG"
        );
        assert_eq!(
            reference("Juan 3:16", Some("16a"), Some("RVG")),
            "Juan 3:16 · RVG"
        );
        assert_eq!(reference("Salmos 23", Some("4"), None), "Salmos 23:4");
        assert_eq!(reference("Juan 3:16", None, Some("RVG")), "Juan 3:16 · RVG");
        // Over two chapters the verse alone wouldn't say which.
        assert_eq!(
            reference("Juan 3:36-4:2", Some("1"), Some("RVG")),
            "Juan 3:36-4:2 · RVG"
        );
    }

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
