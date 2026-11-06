//! The chorquest desktop app: an operator dashboard and a fullscreen stage
//! window for the projector, both following one [`Engine`].
//!
//! Set `CHORQUEST_SCRIPT=<file>` to drive the engine from a script and exit
//! after its last line (see [`script`]). `CHORQUEST_SHOT=<program>` is run
//! for each `shot NAME` in it.

mod script;
mod stage;
mod youtube;

use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use chorquest_engine::{
    Action, Engine, Media, MediaReport, Playback, Resource, SceneId, StageEvent, StageFeed,
    StageState,
};
use iced::futures::{SinkExt, Stream, StreamExt};
use iced::widget::{Space, column, container, image, stack, text};
use iced::{Color, ContentFit, Element, Fill, Subscription, Task, window};
use iced_video_player::VideoPlayer;

use crate::script::Script;
use crate::stage::{Content, Loaded, Stage};
use crate::youtube::{Downloads, YoutubeVideo};

/// The Wayland app id of the audience window. A Hyprland rule matches it to
/// send the window to the projector.
const STAGE_APP_ID: &str = "chorquest-stage";
const DASHBOARD_APP_ID: &str = "chorquest-dashboard";

fn main() -> ExitCode {
    log("starting");
    let script = match load_script() {
        Ok(script) => script,
        Err(err) => {
            log(format!("CHORQUEST_SCRIPT: {err}"));
            return ExitCode::FAILURE;
        }
    };
    let engine = Shared(Arc::new(Engine::new()));
    let downloads = Shared(Downloads::new(stage::cache_dir().join("youtube")));
    let result = iced::daemon(
        move || App::boot(engine.clone(), downloads.clone(), script.clone()),
        App::update,
        App::view,
    )
    .title(App::title)
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
    let Some(path) = std::env::var_os("CHORQUEST_SCRIPT") else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    let source =
        std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let shooter = std::env::var_os("CHORQUEST_SHOT").map(PathBuf::from);
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
    /// Time to redraw download progress.
    Downloading,
    /// The script performed a command. The plan may have changed, which the
    /// stage feed doesn't announce.
    Acted,
    ScriptDone,
    Closed(window::Id),
}

struct App {
    engine: Shared<Engine>,
    downloads: Shared<Downloads>,
    script: Option<Script>,
    stage_window: window::Id,
    /// The stage window's mirror, built only from the engine's feed.
    stage: StageState,
    media: Stage,
}

impl App {
    fn boot(
        engine: Shared<Engine>,
        downloads: Shared<Downloads>,
        script: Option<Script>,
    ) -> (App, Task<Message>) {
        let (_, open_dashboard) = window::open(window::Settings {
            size: iced::Size::new(960.0, 640.0),
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
            downloads,
            script,
            stage_window,
            stage: StageState::default(),
        };
        // Dashboard first: on a single monitor, Hyprland drops a fullscreen
        // window back to tiled when another window maps on its workspace.
        (app, open_dashboard.discard().chain(open_stage.discard()))
    }

    fn title(&self, window: window::Id) -> String {
        if window == self.stage_window {
            "chorquest stage".into()
        } else {
            "chorquest".into()
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
            Message::Downloading => {}
            Message::Acted => return self.preload(),
            Message::ScriptDone => {
                log("script finished, exiting");
                return iced::exit();
            }
            Message::Closed(window) => {
                log(format!("window {window:?} closed, exiting"));
                return iced::exit();
            }
        }
        Task::none()
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

    fn dashboard_view(&self) -> Element<'_, Message> {
        let engine = &self.engine.0;
        let plan = engine.plan();
        let current = engine.current_step().map(|step| step.id);
        let program = match &self.stage.program {
            Some(scene) => format!(
                "Program: {} ({}){}",
                scene.resource.title,
                describe(&scene.media),
                match scene.media {
                    // The failure already says why.
                    Media::Failed { .. } => String::new(),
                    _ => self.download_note(&scene.resource),
                }
            ),
            None => "Program: nothing".into(),
        };
        let preview = match &self.stage.preview {
            Some(scene) => format!(
                "Preview: {}{}",
                scene.resource.title,
                self.download_note(&scene.resource)
            ),
            None => "Preview: nothing".into(),
        };
        let blackout = if self.stage.blackout {
            "Blackout: on"
        } else {
            "Blackout: off"
        };
        let steps = plan.steps().iter().map(|step| {
            let marker = if Some(step.id) == current { ">" } else { " " };
            let note = match &step.action {
                Action::Show { resource } => self.download_note(resource),
                _ => String::new(),
            };
            text(format!("{marker} {}{note}", step.label))
                .size(20)
                .into()
        });
        column![
            text("chorquest").size(32),
            text(program).size(20),
            text(preview).size(20),
            text(blackout).size(20),
            text(format!("Plan: {}", plan.title)).size(24),
            column(steps).spacing(4),
        ]
        .spacing(12)
        .padding(24)
        .into()
    }

    /// A YouTube video's download state, e.g. " [downloading 42%]", or
    /// nothing for anything else.
    fn download_note(&self, resource: &Resource) -> String {
        YoutubeVideo::of(resource)
            .and_then(|video| self.downloads.0.state(&video.id))
            .map_or_else(String::new, |download| format!(" [{download}]"))
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            Subscription::run_with(self.engine.clone(), stage_feed),
            window::close_events().map(Message::Closed),
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
        if self.downloads.0.busy() {
            subscriptions
                .push(iced::time::every(Duration::from_millis(250)).map(|_| Message::Downloading));
        }
        if let Some(script) = &self.script {
            subscriptions.push(Subscription::run_with(
                (self.engine.clone(), self.downloads.clone(), script.clone()),
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

/// The program's media for the operator, e.g. "frame 2 of 3" or "playing 0:03 / 0:06".
fn describe(media: &Media) -> String {
    let clock = |d: Duration| format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60);
    match media {
        Media::Frames { frame, count } => match count {
            Some(count) => format!("frame {} of {count}", frame + 1),
            None => format!("frame {}", frame + 1),
        },
        Media::Timed(timed) => {
            let state = match timed.playback {
                Playback::Paused => "paused",
                Playback::Playing => "playing",
                Playback::Ended => "ended",
            };
            let duration = timed.duration.map_or("?".into(), clock);
            format!("{state} {} / {duration}", clock(timed.position))
        }
        Media::Failed { reason } => format!("failed: {reason}"),
    }
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
    (engine, downloads, script): &(Shared<Engine>, Shared<Downloads>, Script),
) -> impl Stream<Item = Message> + use<> {
    let engine = engine.0.clone();
    let downloads = downloads.0.clone();
    let script = script.clone();
    iced::stream::channel(64, async move |mut out| {
        let (acted, mut received) = iced::futures::channel::mpsc::unbounded();
        std::thread::spawn(move || {
            script.run(&engine, &downloads, || {
                let _ = acted.unbounded_send(Message::Acted);
            });
            let _ = acted.unbounded_send(Message::ScriptDone);
        });
        while let Some(message) = received.next().await {
            if out.send(message).await.is_err() {
                break;
            }
        }
    })
}
