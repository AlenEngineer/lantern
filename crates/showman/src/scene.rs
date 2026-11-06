use std::sync::Arc;

use chorquest_resource::{Kind, Location, Resource};
use serde::{Deserialize, Serialize};

/// Playback state of a scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Playback {
    /// Not timed media (image, text, document...).
    Static,
    /// Timed media, stopped or paused.
    Paused,
    /// Timed media, playing.
    Playing,
}

/// A resource as it is being presented.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// What is presented.
    pub resource: Arc<Resource>,
    /// Current frame, from 0.
    pub frame: usize,
    /// Total frames, if known. Inline text knows its own; for documents and
    /// decks the stage window reports it once it has opened the file
    /// (see [`Showman::set_frame_count`](crate::Showman::set_frame_count)).
    /// Until then the scene behaves as a single frame.
    pub frame_count: Option<usize>,
    /// Playback state.
    pub playback: Playback,
}

impl Scene {
    /// A scene at its first frame. Timed media starts paused: pressing play
    /// is always a deliberate act.
    pub fn new(resource: Arc<Resource>) -> Self {
        let frame_count = match (&resource.location, &resource.kind) {
            (Location::Inline(body), _) => Some(text_frames(body).len().max(1)),
            (_, Kind::Image | Kind::Video | Kind::Audio | Kind::Web) => Some(1),
            _ => None,
        };
        let playback = if resource.kind.is_timed() {
            Playback::Paused
        } else {
            Playback::Static
        };
        Self {
            resource,
            frame: 0,
            frame_count,
            playback,
        }
    }

    /// Whether there is a frame after the current one.
    pub fn has_next_frame(&self) -> bool {
        self.frame + 1 < self.frame_count.unwrap_or(1)
    }

    /// The text of the current frame, for inline text.
    pub fn text(&self) -> Option<String> {
        let body = self.resource.location.as_text()?;
        text_frames(body).into_iter().nth(self.frame)
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
