//! The chorquest desktop app: an operator dashboard and a fullscreen stage
//! window for the projector, both following one [`Engine`].
//!
//! Set `CHORQUEST_SCRIPT=<file>` to drive the engine from a script and exit
//! after its last line (see [`script`]). `CHORQUEST_SHOT=<program>` is run
//! for each `shot NAME` in it.

mod script;

use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use chorquest_engine::{Engine, StageEvent, StageFeed, StageState};
use iced::futures::{SinkExt, Stream, StreamExt};
use iced::widget::{Space, column, container, text};
use iced::{Color, Element, Fill, Subscription, Task, window};

use crate::script::Script;

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
    let result = iced::daemon(
        move || App::boot(engine.clone(), script.clone()),
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
    Script::parse(&source, shooter)
        .map(Some)
        .map_err(|err| format!("{}: {err}", path.display()))
}

/// Writes one timestamped line to stderr, which the e2e harness keeps as `app.log`.
fn log(message: impl AsRef<str>) {
    static START: OnceLock<Instant> = OnceLock::new();
    let elapsed = START.get_or_init(Instant::now).elapsed().as_secs_f64();
    eprintln!("[{elapsed:8.3}] {}", message.as_ref());
}

/// The engine, hashed by identity so it can key a [`Subscription`].
#[derive(Clone)]
struct Shared(Arc<Engine>);

impl Hash for Shared {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

#[derive(Debug, Clone)]
enum Message {
    /// The stage as it was when the feed started. Always the feed's first message.
    Feed(StageState),
    Stage(StageEvent),
    ScriptDone,
    Closed(window::Id),
}

struct App {
    engine: Shared,
    script: Option<Script>,
    stage_window: window::Id,
    /// The stage window's mirror, built only from the engine's feed.
    stage: StageState,
}

impl App {
    fn boot(engine: Shared, script: Option<Script>) -> (App, Task<Message>) {
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
            engine,
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
            Message::Feed(state) => self.stage = state,
            Message::Stage(event) => {
                self.stage.apply(&event);
            }
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

    fn view(&self, window: window::Id) -> Element<'_, Message> {
        if window == self.stage_window {
            self.stage_view()
        } else {
            self.dashboard_view()
        }
    }

    fn stage_view(&self) -> Element<'_, Message> {
        let scene = self.stage.program.as_ref().filter(|_| !self.stage.blackout);
        let content: Element<'_, Message> = match scene {
            None => Space::new().into(),
            Some(scene) => text(scene.text().unwrap_or_else(|| scene.resource.title.clone()))
                .size(64)
                .color(Color::WHITE)
                .align_x(iced::alignment::Horizontal::Center)
                .into(),
        };
        container(content)
            .center(Fill)
            .style(|_| container::Style::default().background(Color::BLACK))
            .into()
    }

    fn dashboard_view(&self) -> Element<'_, Message> {
        let engine = &self.engine.0;
        let plan = engine.plan();
        let current = engine.current_step().map(|step| step.id);
        let program = match &self.stage.program {
            Some(scene) => format!(
                "Program: {} (frame {})",
                scene.resource.title,
                scene.frame() + 1
            ),
            None => "Program: nothing".into(),
        };
        let blackout = if self.stage.blackout {
            "Blackout: on"
        } else {
            "Blackout: off"
        };
        let steps = plan.steps().iter().map(|step| {
            let marker = if Some(step.id) == current { ">" } else { " " };
            text(format!("{marker} {}", step.label)).size(20).into()
        });
        column![
            text("chorquest").size(32),
            text(program).size(20),
            text(blackout).size(20),
            text(format!("Plan: {}", plan.title)).size(24),
            column(steps).spacing(4),
        ]
        .spacing(12)
        .padding(24)
        .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            Subscription::run_with(self.engine.clone(), stage_feed),
            window::close_events().map(Message::Closed),
        ];
        if let Some(script) = &self.script {
            subscriptions.push(Subscription::run_with(
                (self.engine.clone(), script.clone()),
                run_script,
            ));
        }
        Subscription::batch(subscriptions)
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
fn stage_feed(engine: &Shared) -> impl Stream<Item = Message> + use<> {
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
/// `shot`), and reports when it is done.
fn run_script((engine, script): &(Shared, Script)) -> impl Stream<Item = Message> + use<> {
    let engine = engine.0.clone();
    let script = script.clone();
    iced::stream::channel(1, async move |mut out| {
        let (done, finished) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            script.run(&engine);
            let _ = done.send(());
        });
        let _ = finished.await;
        let _ = out.send(Message::ScriptDone).await;
    })
}
