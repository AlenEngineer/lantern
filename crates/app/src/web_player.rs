//! YouTube's own player: the `lantern-web` program, which shows a video
//! through YouTube's IFrame Player API in a window of its own over the
//! stage. Lantern starts it the first time a video plays that way, keeps
//! it for the next, and steers it with [`Command`]s; it answers with
//! [`Event`]s, read here on a thread of their own.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{Receiver, channel};

pub use lantern_web_protocol::{Command, Event};

/// The running `lantern-web`.
pub struct WebPlayer {
    child: Child,
    stdin: ChildStdin,
    events: Receiver<Event>,
    /// The last volume and opacity sent, so a fade's every tick doesn't
    /// say the same again.
    volume: Option<f64>,
    opacity: Option<f64>,
}

impl WebPlayer {
    /// Starts `lantern-web`, from beside Lantern or the `PATH`.
    pub fn start() -> Result<WebPlayer, String> {
        Self::start_with(&[])
    }

    /// Starts the picker: YouTube's own site at `address`, its cookies
    /// kept in `profile`.
    pub fn pick(address: &str, profile: &std::path::Path) -> Result<WebPlayer, String> {
        let profile = profile.to_string_lossy();
        Self::start_with(&["--pick", address, "--profile", &profile])
    }

    fn start_with(args: &[&str]) -> Result<WebPlayer, String> {
        let mut child = crate::programs::command("lantern-web")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|err| format!("YouTube's player (lantern-web) won't start: {err}"))?;
        let stdin = child.stdin.take().ok_or("lantern-web has no stdin")?;
        let stdout = child.stdout.take().ok_or("lantern-web has no stdout")?;
        let (send, events) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                match serde_json::from_str::<Event>(&line) {
                    Ok(event) => {
                        if send.send(event).is_err() {
                            break;
                        }
                    }
                    Err(err) => crate::log(format!("web player: odd line ({err}): {line}")),
                }
            }
        });
        crate::log("web player: started");
        Ok(WebPlayer {
            child,
            stdin,
            events,
            volume: None,
            opacity: None,
        })
    }

    /// Its process id, to find its window.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Whether the program is still running.
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Tells the player `command`. Volume and opacity only when they change.
    pub fn send(&mut self, command: Command) {
        match command {
            Command::Volume { level } if self.volume == Some(level) => return,
            Command::Opacity { level } if self.opacity == Some(level) => return,
            Command::Volume { level } => self.volume = Some(level),
            Command::Opacity { level } => self.opacity = Some(level),
            Command::Load { .. } => {
                self.volume = None;
                self.opacity = None;
            }
            _ => {}
        }
        let line = match serde_json::to_string(&command) {
            Ok(line) => line,
            Err(err) => return crate::log(format!("web player: {err}")),
        };
        if let Err(err) = writeln!(self.stdin, "{line}").and_then(|()| self.stdin.flush()) {
            crate::log(format!("web player: can't tell it {line}: {err}"));
        }
    }

    /// What it said since last asked.
    pub fn events(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

impl Drop for WebPlayer {
    fn drop(&mut self) {
        self.send(Command::Quit);
        // Closing its stdin ends it as well; this is for one that hangs.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
