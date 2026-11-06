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
mod library;
mod script;
mod search;
mod stage;
mod youtube;

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
    Action, Engine, Media, MediaReport, Playback, ResourceId, SceneId, SearchEvent, SearchStream,
    StageEvent, StageFeed, StageState, StepId,
};

use crate::dashboard::{Op, RESULTS, SEARCH_BOX};
use crate::library::Library;
use crate::script::{Script, Ui};
use crate::search::Search;
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
    let (config, config_status) = config::load(|key| std::env::var(key).ok());
    log(config_status.to_string());
    let config_status = Arc::new(config_status.to_string());
    let engine = Shared(Arc::new(Engine::new()));
    let library = Shared(Library::start(engine.0.clone(), config));
    let downloads = Shared(Downloads::new(stage::cache_dir().join("youtube")));
    let result = iced::daemon(
        move || {
            App::boot(
                engine.clone(),
                library.clone(),
                config_status.clone(),
                downloads.clone(),
                script.clone(),
            )
        },
        App::update,
        App::view,
    )
    .title(App::title)
    .theme(|_: &App, _| dashboard::theme())
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
    Op(Op),
    GoTo(StepId),
    Remove(StepId),
    /// The timeline is being dragged to this many seconds.
    Scrub(f64),
    /// The timeline was let go.
    Scrubbed,
}

struct App {
    engine: Shared<Engine>,
    library: Shared<Library>,
    /// Which config file was read, or why none was.
    config_status: Arc<String>,
    downloads: Shared<Downloads>,
    script: Option<Script>,
    stage_window: window::Id,
    /// The stage window's mirror, built only from the engine's feed.
    stage: StageState,
    media: Stage,
    search: Search,
    /// Where the timeline is being dragged, until it is let go.
    scrub: Option<f64>,
}

impl App {
    fn boot(
        engine: Shared<Engine>,
        library: Shared<Library>,
        config_status: Arc<String>,
        downloads: Shared<Downloads>,
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
        let app = App {
            media: Stage::new(downloads.0.clone()),
            engine,
            library,
            config_status,
            downloads,
            script,
            stage_window,
            stage: StageState::default(),
            search: Search::default(),
            scrub: None,
        };
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
                    Ui::Type(query) => self.query(query),
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
                    Ui::Press(op) => self.op(op),
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
            Message::Op(op) => return self.op(op),
            Message::GoTo(step) => {
                if let Err(err) = self.engine.0.go_to(step) {
                    log(format!("dashboard: {err}"));
                }
                return self.preload();
            }
            Message::Remove(step) => {
                self.engine.0.edit_plan(|plan| plan.remove(step));
                return self.preload();
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
        let Some(edit) = self.search.edit(query, &self.engine.0) else {
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
                if self.search.picked().is_none()
                    && let Some(first) = self.search.pick_nth(0)
                {
                    engine.preview(first);
                    return self.scroll_to_pick();
                }
                return self.op(Op::Take);
            }
            Op::Take => {
                if engine.take() {
                    self.search.unpick();
                }
            }
            Op::AddToPlan => {
                if let Some(scene) = engine.stage().preview {
                    engine.add_after_current(&scene.resource);
                    return self.preload();
                }
            }
            Op::Next => {
                engine.next();
                return self.preload();
            }
            Op::Prev => {
                engine.prev();
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
            Op::FocusSearch => {
                return Task::batch([
                    operation::focus(SEARCH_BOX),
                    operation::select_all(SEARCH_BOX),
                ]);
            }
        }
        Task::none()
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
                Some(Content::Video(video)) => VideoPlayer::new(video)
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
            iced::event::listen_with(dashboard::key_event),
        ];
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
