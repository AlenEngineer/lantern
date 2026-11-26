//! What lantern and its YouTube player window say to each other.
//!
//! The player is a program of its own, `lantern-web`: a window with a web
//! view that plays a video through YouTube's IFrame Player API, the way
//! YouTube's terms allow. The app starts it, writes [`Command`]s to its
//! stdin and reads [`Event`]s from its stdout, one JSON object per line.
//! When its stdin closes (the app quit), it quits too.
//!
//! Started with `--pick <address>` it is instead a window on the
//! operator's screen showing YouTube's own site at that address (its search
//! for what was typed), with a strip of lantern's above it. A video the
//! operator clicks there isn't opened: it is told as [`Event::Picked`].

use serde::{Deserialize, Serialize};

/// What the app asks the player to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Command {
    /// Get `id` ready from `start` seconds. It starts playing when the
    /// window is shown, if `play`; YouTube allows playing on its own only
    /// once the player is on screen.
    Load {
        id: String,
        start: f64,
        play: bool,
    },
    Play,
    Pause,
    /// Jump to this many seconds in.
    Seek {
        seconds: f64,
    },
    /// Loudness, 0 to 1.
    Volume {
        level: f64,
    },
    /// How much of the picture shows over black, 0 to 1: fades and blackout.
    Opacity {
        level: f64,
    },
    /// Full screen on the output with this name, or failing that the one
    /// holding the point `at` (logical pixels; on Wayland the window
    /// system's names aren't the outputs' names), or the current one.
    Show {
        #[serde(default)]
        output: Option<String>,
        #[serde(default)]
        at: Option<(i32, i32)>,
    },
    Hide,
    /// Picking: what lantern's strip above YouTube's page says.
    Strip {
        text: String,
    },
    Quit,
}

/// What the player tells the app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The page and YouTube's script loaded: commands can come.
    Ready,
    /// The video's length, in seconds, once known.
    Timeline {
        seconds: f64,
    },
    /// Where it is, in seconds, a few times a second while playing.
    Position {
        seconds: f64,
    },
    Playing,
    Paused,
    Ended,
    /// Picking: the operator clicked this video (its 11-character id).
    Picked {
        id: String,
    },
    /// Picking: the operator is done, by the strip's Done or by closing
    /// the window.
    Done,
    /// `Show` named an output that isn't there, so the window stays hidden
    /// rather than cover the operator's screen.
    NoOutput,
    /// YouTube's error code: 2 a bad id, 5 the player failed, 100 not
    /// found or private, 101 and 150 its owner doesn't allow embedding.
    Error {
        code: u32,
    },
}

/// Whether `id` looks like a YouTube video id: 11 of `A-Z a-z 0-9 _ -`.
/// What the picker's page says is checked with it before it's believed.
pub fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl Event {
    /// Whether YouTube refused to play the video outside its site.
    pub fn not_embeddable(&self) -> bool {
        matches!(
            self,
            Event::Error {
                code: 101 | 150 | 153
            }
        )
    }
}

impl Command {
    /// The JavaScript that carries the command out in the player page.
    pub fn script(&self) -> String {
        let json = |value: &str| serde_json::to_string(value).unwrap_or_default();
        match self {
            Command::Load { id, start, play } => {
                format!("lantern.load({}, {start}, {play})", json(id))
            }
            Command::Play => "lantern.play()".into(),
            Command::Pause => "lantern.pause()".into(),
            Command::Seek { seconds } => format!("lantern.seek({seconds})"),
            Command::Volume { level } => format!("lantern.volume({})", level.clamp(0.0, 1.0)),
            Command::Opacity { level } => {
                format!("lantern.opacity({})", level.clamp(0.0, 1.0))
            }
            Command::Show { .. } => "lantern.shown()".into(),
            Command::Hide => "lantern.hidden()".into(),
            Command::Strip { text } => format!("say({})", json(text)),
            Command::Quit => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_and_events_read_back_as_written() {
        let commands = [
            Command::Load {
                id: "dQw4w9WgXcQ".into(),
                start: 12.5,
                play: true,
            },
            Command::Seek { seconds: 3.0 },
            Command::Show {
                output: Some("HDMI-A-1".into()),
                at: Some((1920, 0)),
            },
            Command::Hide,
        ];
        for command in commands {
            let line = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&line).unwrap(), command);
        }
        let event: Event = serde_json::from_str(r#"{"event":"timeline","seconds":212.4}"#).unwrap();
        assert_eq!(event, Event::Timeline { seconds: 212.4 });
    }

    #[test]
    fn an_id_cannot_break_out_of_the_script() {
        let load = Command::Load {
            id: "a\");evil(\"".into(),
            start: 0.0,
            play: false,
        };
        assert_eq!(load.script(), r#"lantern.load("a\");evil(\"", 0, false)"#);
    }

    #[test]
    fn levels_stay_between_zero_and_one() {
        assert_eq!(Command::Volume { level: 1.7 }.script(), "lantern.volume(1)");
        assert_eq!(
            Command::Opacity { level: -0.2 }.script(),
            "lantern.opacity(0)"
        );
    }

    #[test]
    fn picking_reads_back_and_ids_are_checked() {
        for event in [
            Event::Picked {
                id: "jNQXAC9IVRw".into(),
            },
            Event::Done,
        ] {
            let line = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), event);
        }
        let strip = Command::Strip {
            text: "2 picked · added \"Himno\"".into(),
        };
        assert_eq!(strip.script(), r#"say("2 picked · added \"Himno\"")"#);
        assert!(is_video_id("jNQXAC9IVRw"));
        assert!(is_video_id("a-b_c-d_e-f"));
        assert!(!is_video_id("jNQXAC9IVR"), "too short");
        assert!(!is_video_id("jNQXAC9IVRw\");alert(1)"));
    }

    #[test]
    fn embedding_refused_is_told_apart() {
        assert!(Event::Error { code: 150 }.not_embeddable());
        assert!(!Event::Error { code: 100 }.not_embeddable());
    }
}
