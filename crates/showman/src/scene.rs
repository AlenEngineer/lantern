use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use chorquest_resource::{Kind, Location, Resource};
use serde::{Deserialize, Serialize};

/// Names one scene for its whole life.
///
/// The [`Showman`](crate::Showman) is the only code that makes ids, and it
/// keeps two rules:
/// - an id is never reused, even after the stage is cleared;
/// - an id never changes slot. [`take`](crate::Showman::take) gives the new
///   program scene a fresh id, so a report meant for the preview can never
///   land on program.
///
/// So a report tagged with an id reaches the scene it was made for, or
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SceneId(u64);

impl SceneId {
    pub(crate) fn new(raw: u64) -> Self {
        Self(raw)
    }
}

impl fmt::Display for SceneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "scene #{}", self.0)
    }
}

/// A resource as it is being presented, in program or in preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// Which scene this is. Renderers tag their reports with it.
    pub id: SceneId,
    /// What is presented.
    pub resource: Arc<Resource>,
    /// How it is presented right now.
    pub media: Media,
}

/// How a scene is presented. Exactly one shape at a time, so "playing, on
/// frame 3 of 5" can't be represented.
///
/// The engine guesses the shape when the scene is made. The renderer is the
/// authority, and its [`MediaReport`]s replace the guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Media {
    /// Stanzas, pages or slides. A still image or a web page is one frame.
    Frames {
        /// Current frame, from 0. Below `count` whenever `count` is known.
        frame: usize,
        /// `None` until the stage reports it (documents, decks). Until then
        /// the scene behaves as a single frame.
        count: Option<NonZeroUsize>,
    },
    /// Audio or video on a timeline.
    Timed(Timed),
    /// The stage reported that it can't present the scene. There are no
    /// frames to step through and nothing to play. A later `Frames` or
    /// `Timeline` report (a retry that worked) replaces it.
    Failed {
        /// What the stage said, for the operator.
        reason: String,
    },
}

/// The timeline of a timed scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timed {
    /// Starts `Paused`, because pressing play is always a deliberate act.
    pub playback: Playback,
    /// The last position the stage reported, or the target of the last seek,
    /// whichever came later. While playing it moves only when the stage
    /// reports, so a dashboard may extrapolate between reports.
    #[serde(rename = "position_ms", with = "millis")]
    pub position: Duration,
    /// `None` until the stage reports it, and for streams without an end.
    #[serde(rename = "duration_ms", with = "millis_opt")]
    pub duration: Option<Duration>,
}

/// Where a timed scene is in its life.
///
/// The operator moves between `Paused` and `Playing`. Only the stage's
/// [`MediaReport::Ended`] reaches `Ended`. From `Ended`, `play` replays from
/// the start and a seek returns to `Paused`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Playback {
    /// Paused, or not started. A scene still loading is paused.
    Paused,
    /// Playing.
    Playing,
    /// Reached the end.
    Ended,
}

/// What a renderer learned about one scene's media, sent with the scene's id.
///
/// Every report states an absolute fact, so sending one twice changes
/// nothing and a lost report is fixed by the next one. A report for a scene
/// that has left the stage is ignored.
///
/// Serializable so a webview stage can send reports across a process
/// boundary. Externally tagged, because two variants wrap a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaReport {
    /// The stage presents the scene as this many frames: the pages of a PDF
    /// or the slides of a deck. The current frame is clamped to the count.
    /// Ignored for timed scenes (a stage that can't play one reports
    /// `Failed`) and for inline text, whose frames come from
    /// [`text_frames`].
    Frames(NonZeroUsize),
    /// The stage plays the scene on a timeline of this length. A frames
    /// scene becomes timed and starts paused, so the stage must hold it
    /// paused until a `Playback` event says otherwise. Ignored for inline
    /// text.
    Timeline {
        /// `None` for a stream without an end.
        #[serde(rename = "duration_ms", with = "millis_opt")]
        duration: Option<Duration>,
    },
    /// Where playback is now. Send it about four times a second while
    /// playing, and once after a seek or a pause settles. Ignored once the
    /// scene has ended, so a report already in flight at the end of the
    /// video can't pull it back.
    Position(#[serde(with = "millis")] Duration),
    /// Playback reached the end.
    Ended,
    /// The stage can't present the scene. Its stage window shows black, and
    /// the reason goes to the operator. Play, pause and seek are refused
    /// from then on.
    Failed {
        /// Shown to the operator.
        reason: String,
    },
}

impl Scene {
    pub(crate) fn new(id: SceneId, resource: Arc<Resource>) -> Self {
        let media = Media::guess(&resource);
        Self {
            id,
            resource,
            media,
        }
    }

    /// The current frame. 0 for timed and failed scenes.
    pub fn frame(&self) -> usize {
        match self.media {
            Media::Frames { frame, .. } => frame,
            Media::Timed(_) | Media::Failed { .. } => 0,
        }
    }

    /// Whether there is a frame after the current one. An unknown count
    /// behaves as one frame.
    pub fn has_next_frame(&self) -> bool {
        match self.media {
            Media::Frames { frame, count } => frame + 1 < count.map_or(1, NonZeroUsize::get),
            Media::Timed(_) | Media::Failed { .. } => false,
        }
    }

    /// The text of the current frame, for inline text.
    pub fn text(&self) -> Option<String> {
        let body = self.resource.location.as_text()?;
        text_frames(body).into_iter().nth(self.frame())
    }

    /// The timeline, for timed scenes.
    pub fn timed(&self) -> Option<&Timed> {
        match &self.media {
            Media::Timed(timed) => Some(timed),
            Media::Frames { .. } | Media::Failed { .. } => None,
        }
    }
}

impl Media {
    /// The engine's guess before any renderer has spoken. The one place that
    /// decides frames versus timeline from a resource.
    fn guess(resource: &Resource) -> Media {
        match (&resource.location, &resource.kind) {
            (Location::Inline(body), _) => Media::Frames {
                frame: 0,
                count: Some(
                    NonZeroUsize::new(text_frames(body).len()).unwrap_or(NonZeroUsize::MIN),
                ),
            },
            (_, Kind::Video | Kind::Audio) => Media::Timed(Timed::start(None)),
            (_, Kind::Image | Kind::Web) => Media::Frames {
                frame: 0,
                count: Some(NonZeroUsize::MIN),
            },
            (_, Kind::Document | Kind::Presentation | Kind::Text | Kind::Other(_)) => {
                Media::Frames {
                    frame: 0,
                    count: None,
                }
            }
        }
    }

    /// What `report` turns this media into, or `None` if the report doesn't
    /// apply to it. The `Showman` broadcasts the result, so mirrors never
    /// re-run this policy.
    pub(crate) fn after(&self, resource: &Resource, report: &MediaReport) -> Option<Media> {
        use MediaReport as R;
        let inline = resource.location.as_text().is_some();
        match (self, report) {
            // The engine splits inline text itself, so the stage has no say in its shape.
            (_, R::Frames(_) | R::Timeline { .. }) if inline => None,

            (Media::Frames { frame, .. }, R::Frames(count)) => Some(Media::Frames {
                frame: (*frame).min(count.get() - 1),
                count: Some(*count),
            }),
            (Media::Failed { .. }, R::Frames(count)) => Some(Media::Frames {
                frame: 0,
                count: Some(*count),
            }),
            (Media::Timed(_), R::Frames(_)) => None,

            (Media::Timed(timed), R::Timeline { duration }) => Some(Media::Timed(Timed {
                duration: *duration,
                ..*timed
            })),
            (Media::Frames { .. } | Media::Failed { .. }, R::Timeline { duration }) => {
                Some(Media::Timed(Timed::start(*duration)))
            }

            (Media::Timed(timed), R::Position(position)) if timed.playback != Playback::Ended => {
                Some(Media::Timed(Timed {
                    position: *position,
                    ..*timed
                }))
            }
            (Media::Timed(timed), R::Ended) => Some(Media::Timed(Timed {
                playback: Playback::Ended,
                position: timed.duration.unwrap_or(timed.position),
                ..*timed
            })),

            (_, R::Failed { reason }) => Some(Media::Failed {
                reason: reason.clone(),
            }),

            (_, R::Position(_) | R::Ended) => None,
        }
    }
}

impl Timed {
    fn start(duration: Option<Duration>) -> Self {
        Self {
            playback: Playback::Paused,
            position: Duration::ZERO,
            duration,
        }
    }

    /// Moves the playhead. A timeline moved off its end is no longer ended.
    pub(crate) fn seek(&mut self, position: Duration) {
        self.position = position;
        if self.playback == Playback::Ended {
            self.playback = Playback::Paused;
        }
    }
}

/// Splits text into slides at blank lines. Lines within a slide are kept;
/// surrounding whitespace is trimmed.
pub fn text_frames(body: &str) -> Vec<String> {
    let mut frames = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in body.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                frames.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line.trim_end());
        }
    }
    if !current.is_empty() {
        frames.push(current.join("\n"));
    }
    frames
}

/// Durations cross the wire as whole milliseconds.
pub(crate) mod millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        to_millis(*value).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        u64::deserialize(deserializer).map(Duration::from_millis)
    }

    pub(super) fn to_millis(value: Duration) -> u64 {
        u64::try_from(value.as_millis()).unwrap_or(u64::MAX)
    }
}

/// [`millis`] for `Option<Duration>`.
pub(crate) mod millis_opt {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<Duration>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.map(super::millis::to_millis).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Duration>, D::Error> {
        Option::<u64>::deserialize(deserializer).map(|ms| ms.map(Duration::from_millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_blank_lines() {
        let body = "Line 1\nLine 2\r\n\r\n   \nChorus\n\n";
        assert_eq!(text_frames(body), ["Line 1\nLine 2", "Chorus"]);
        assert!(text_frames("\n \n").is_empty());
    }
}
