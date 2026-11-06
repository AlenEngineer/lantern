use std::fmt;

use serde::{Deserialize, Serialize};

/// What a resource *is*, from the point of view of presenting it.
///
/// The kind decides how the stage renders something; where it came from is
/// the [`Source`](crate::Source)'s business. A YouTube video and an `.mp4` on
/// disk are both [`Kind::Video`]; a hymn and a Bible verse are both
/// [`Kind::Text`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Still pictures.
    Image,
    /// Moving pictures, usually with sound.
    Video,
    /// Sound only.
    Audio,
    /// Paged documents (PDF, Word...).
    Document,
    /// Slide decks (PowerPoint, Impress, Keynote).
    Presentation,
    /// A web page.
    Web,
    /// Plain text shown as slides: lyrics, verses, announcements.
    Text,
    /// Escape hatch for extensions that need a kind the core doesn't know.
    Other(String),
}

impl Kind {
    /// Guesses the kind from a file extension (case-insensitive, without the dot).
    pub fn from_extension(ext: &str) -> Option<Kind> {
        let kind = match ext.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "tif" | "tiff" | "heic"
            | "avif" => Kind::Image,
            "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" | "wmv" | "mpg" | "mpeg" => Kind::Video,
            "mp3" | "wav" | "flac" | "ogg" | "oga" | "opus" | "m4a" | "aac" | "wma" => Kind::Audio,
            "pdf" | "doc" | "docx" | "odt" | "rtf" => Kind::Document,
            "ppt" | "pptx" | "pps" | "ppsx" | "odp" | "key" => Kind::Presentation,
            "html" | "htm" => Kind::Web,
            "txt" | "md" => Kind::Text,
            _ => return None,
        };
        Some(kind)
    }

    /// Whether the kind has a timeline (play, pause, seek).
    pub fn is_timed(&self) -> bool {
        matches!(self, Kind::Video | Kind::Audio)
    }

    /// A short lowercase name, e.g. `"video"`.
    pub fn name(&self) -> &str {
        match self {
            Kind::Image => "image",
            Kind::Video => "video",
            Kind::Audio => "audio",
            Kind::Document => "document",
            Kind::Presentation => "presentation",
            Kind::Web => "web",
            Kind::Text => "text",
            Kind::Other(name) => name,
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_case_insensitive() {
        assert_eq!(Kind::from_extension("MP4"), Some(Kind::Video));
        assert_eq!(Kind::from_extension("pptx"), Some(Kind::Presentation));
        assert_eq!(Kind::from_extension("Jpeg"), Some(Kind::Image));
        assert_eq!(Kind::from_extension("exe"), None);
    }
}
