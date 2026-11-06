use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where the content of a resource lives.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Location {
    /// A file on this machine.
    File(PathBuf),
    /// Something reachable by URL: a YouTube video, a web page, a remote image.
    Url(String),
    /// The content itself, for textual resources (lyrics, verses, notes).
    /// Blank lines separate slides.
    Inline(String),
}

impl Location {
    /// The path, if this is a local file.
    pub fn as_path(&self) -> Option<&Path> {
        match self {
            Location::File(path) => Some(path),
            _ => None,
        }
    }

    /// The URL, if this is a remote resource.
    pub fn as_url(&self) -> Option<&str> {
        match self {
            Location::Url(url) => Some(url),
            _ => None,
        }
    }

    /// The text, if the content is inline.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Location::Inline(text) => Some(text),
            _ => None,
        }
    }
}
