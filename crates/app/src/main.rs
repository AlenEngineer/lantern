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
mod fit;
mod keys;
mod length;
mod library;
mod look;
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

/// What a click lands on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Clicked {
    Result(ResourceId),
    Step(StepId),
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
    /// Adds a section, live slot, black screen or note after the selection.
    AddStep(plan_edit::NewStep),
    /// "Set a time" on a section: select it and focus its time field.
    SetTime(StepId),
    /// What a live slot searches when the service reaches it ("!yt", "!b").
    SlotSearch(StepId, String),
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
    /// How slide text is split to fit the stage.
    slides: fit::Splitter,
    /// Thumbnails for results and plan steps.
    thumbs: thumbs::Thumbs,
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
    /// The selected plan step: where new steps go, and what Enter puts on
    /// stage while the search box is empty.
    selected: Option<StepId>,
    /// The last click on a result or step, to tell a double click.
    last_click: Option<(Clicked, Instant)>,
    /// Something just went on stage and nothing's been typed since: the
    /// arrows step its slides even with text in the search box.
    stepping: bool,
    /// The plan step the service was at, to notice arriving at a new one.
    arrived_at: Option<StepId>,
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
            slides: fit::Splitter::new(&config.stage),
            thumbs: thumbs::Thumbs::start(),
            config,
            settings: None,
            home: config::env("HOME").map(PathBuf::from),
            palette_error: None,
            downloads,
            script,
            stage_window,
            stage: StageState::default(),
            search: Search::default(),
            selected: None,
            last_click: None,
            stepping: false,
            arrived_at: None,
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
            .chain(operation::focus(SEARCH_BOX))
            .chain(
                window::size(stage_window).map(move |size| Message::Resized(stage_window, size)),
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
                    Ui::Click(n) => match self.search.pick_nth(n) {
                        Some(resource) => {
                            self.update(Message::Click(Clicked::Result(resource.id.clone())))
                        }
                        None => {
                            log(format!("dashboard: no result {}", n + 1));
                            Task::none()
                        }
                    },
                    Ui::Step(n) => match self.engine.0.plan().steps().get(n) {
                        Some(step) => self.update(Message::Click(Clicked::Step(step.id))),
                        None => {
                            log(format!("dashboard: no plan step {}", n + 1));
                            Task::none()
                        }
                    },
                    Ui::Path(text) => {
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
            Message::Type(typed) => {
                self.stepping = false;
                let query = format!("{}{typed}", self.search.query);
                return self.complete(query);
            }
            Message::CheckPalette => self.check_palette(),
            Message::Settings(edit) => {
                let chosen = self.settings.as_mut().and_then(|draft| draft.edit(edit));
                // A chosen path is longer than what was typed: cursor to its end.
                if let Some(field) = chosen {
                    return operation::move_cursor_to_end(field.id());
                }
            }
            Message::SaveSettings => self.save_settings(),
            Message::Select(step) => {
                self.selected = Some(step);
                if let Some(Action::Show { resource }) = self.cued().map(|s| s.action) {
                    self.search.unpick();
                    self.engine.0.preview(Arc::new(resource));
                } else {
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
                let id = self.engine.0.add(self.selected, label, action);
                self.selected = Some(id);
                return operation::focus(plan_edit::name_id(id));
            }
            Message::SetTime(section) => {
                self.selected = Some(section);
                self.engine.0.clear_preview();
                return operation::focus(plan_edit::time_id(section));
            }
            Message::SlotSearch(id, text) => self.engine.0.edit_plan(|plan| {
                if let Some(Step {
                    action: Action::Live { hint, .. },
                    ..
                }) = plan.get_mut(id)
                {
                    *hint = Some(text).filter(|t| !t.trim().is_empty()).map(Query::new);
                }
            }),
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
            Message::Resized(window, size) => {
                if window == self.stage_window && self.slides.set_stage(size) {
                    self.report_slides();
                }
            }
        }
        Task::none()
    }

    /// Index results now, the live sources once typing pauses.
    fn query(&mut self, query: String) -> Task<Message> {
        self.stepping = false;
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
                    self.selected = None;
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
                    self.selected = Some(engine.add_resource(self.selected, &resource));
                    return self.preload();
                }
            }
            Op::Next | Op::Prev => {
                if op == Op::Next {
                    engine.next();
                } else {
                    engine.prev();
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
                    None => Some(
                        settings::Draft::of(&self.config, self.home.as_deref())
                            .with_fonts(fit::families()),
                    ),
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
                let restyled = self.slides.set_look(&config.stage);
                self.config = config;
                self.config_status = Arc::new(config::Status::Read(path));
                if restyled {
                    self.report_slides();
                }
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
            StageEvent::Program { .. } => {
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

    /// The look's background, with the program on top unless it failed.
    /// Blackout covers the program instead of removing it, so a video
    /// keeps playing and still reports its end; it's black whatever the
    /// look.
    fn stage_view(&self) -> Element<'_, Message> {
        let look = &self.config.stage;
        let Some(scene) = &self.stage.program else {
            return backdrop(look, Space::new());
        };
        let content: Element<'_, Message> = match (&scene.media, self.slides.slide(scene)) {
            (Media::Failed { .. }, _) => Space::new().into(),
            (_, Some(body)) => fit::fit_text(body, look).into(),
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
            stack![backdrop(look, content), black(Space::new())].into()
        } else {
            backdrop(look, content)
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            Subscription::run_with(self.engine.clone(), stage_feed),
            window::close_events().map(Message::Closed),
            window::resize_events().map(|(window, size)| Message::Resized(window, size)),
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
fn set_length(plan: &mut Plan, id: StepId, value: Option<Duration>) {
    let Some(step) = plan.get_mut(id) else { return };
    match &mut step.action {
        Action::Section { budget } => *budget = value,
        _ => step.estimate = value,
    }
}

/// `content` over the look's background: its colour, and its image
/// covering the stage.
pub(crate) fn backdrop<'a>(
    look: &Look,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let colour = fit::color(look.background);
    let content = container(content).center(Fill);
    let layers: Element<'a, Message> = match &look.image {
        Some(path) => stack![
            image(image::Handle::from_path(path))
                .width(Fill)
                .height(Fill)
                .content_fit(ContentFit::Cover),
            content,
        ]
        .into(),
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
