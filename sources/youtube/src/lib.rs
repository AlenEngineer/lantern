//! YouTube search as a lantern [`Source`], answering `!yt`.
//!
//! It runs `yt-dlp` (which must be installed) for each search, so a search
//! takes a few seconds. That's why the source is
//! [on request](Capabilities::on_request): only `!yt amazing grace` asks it,
//! never a plain search.
//!
//! Each result is the same resource a pasted YouTube link becomes
//! ([`lantern_source_link::youtube_video`]), titled with the video's title.
//! The subtitle has the channel and the length, or `live` for a stream.

use std::process::Command;

use lantern_resource::{Capabilities, Query, Resource, Source, SourceError, SourceId};
use lantern_source_link::youtube_video;
use serde::Deserialize;

/// How many videos a search asks for unless told otherwise.
pub const DEFAULT_RESULTS: usize = 10;

/// Searches YouTube with `yt-dlp`.
#[derive(Debug, Clone)]
pub struct YoutubeSource {
    id: SourceId,
    program: String,
    results: usize,
}

impl YoutubeSource {
    /// A source that runs `yt-dlp` from `PATH`.
    pub fn new(id: SourceId) -> Self {
        Self {
            id,
            program: "yt-dlp".into(),
            results: DEFAULT_RESULTS,
        }
    }

    /// Runs this program instead of `yt-dlp`.
    pub fn program(mut self, program: impl Into<String>) -> Self {
        self.program = program.into();
        self
    }

    /// How many videos to ask for (default [`DEFAULT_RESULTS`]).
    pub fn results(mut self, results: usize) -> Self {
        self.results = results;
        self
    }

    /// Turns `yt-dlp --flat-playlist -J` output into resources, in its order.
    pub fn parse(&self, json: &str) -> Result<Vec<Resource>, SourceError> {
        let found: Found =
            serde_json::from_str(json).map_err(|err| SourceError::Other(Box::new(err)))?;
        Ok(found
            .entries
            .into_iter()
            .map(|entry| self.resource(entry))
            .collect())
    }

    fn resource(&self, entry: Entry) -> Resource {
        let mut resource = youtube_video(&self.id, &entry.id);
        let length = if entry.live_status.as_deref() == Some("is_live") {
            Some("live".to_string())
        } else {
            entry.duration.map(|seconds| clock(seconds as u64))
        };
        let subtitle: Vec<String> = entry
            .channel
            .or(entry.uploader)
            .into_iter()
            .chain(length)
            .collect();
        if !subtitle.is_empty() {
            resource.subtitle = Some(subtitle.join(" · "));
        }
        if let Some(title) = entry.title {
            resource.title = title;
        }
        resource
    }
}

/// `yt-dlp`'s answer, keeping what the resources need.
#[derive(Deserialize)]
struct Found {
    #[serde(default)]
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    id: String,
    title: Option<String>,
    /// Seconds. `yt-dlp` writes whole numbers, and sometimes fractions.
    duration: Option<f64>,
    channel: Option<String>,
    uploader: Option<String>,
    live_status: Option<String>,
}

/// `3:05`, or `1:02:03` past an hour.
fn clock(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

impl Source for YoutubeSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn name(&self) -> &str {
        "YouTube"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::ON_REQUEST
    }

    fn bang(&self) -> Option<&str> {
        Some("yt")
    }

    fn search(&self, query: &Query) -> Result<Vec<Resource>, SourceError> {
        let text = query.text.trim();
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let wanted = self.results.min(query.limit);
        let output = Command::new(&self.program)
            .args(["--flat-playlist", "--dump-single-json", "--no-warnings"])
            .arg(format!("ytsearch{wanted}:{text}"))
            .output()
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => {
                    SourceError::Unavailable(format!("{} isn't installed", self.program))
                }
                _ => SourceError::Io(err),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = stderr.lines().rfind(|l| !l.trim().is_empty());
            return Err(SourceError::Unavailable(
                reason.unwrap_or("yt-dlp failed").trim().to_string(),
            ));
        }
        self.parse(&String::from_utf8_lossy(&output.stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> YoutubeSource {
        YoutubeSource::new(SourceId::new("youtube").unwrap())
    }

    #[test]
    fn results_are_youtube_videos_titled_with_channel_and_length() {
        let found = source()
            .parse(include_str!("../fixtures/search.json"))
            .unwrap();
        assert_eq!(found.len(), 4);
        let first = &found[0];
        assert_eq!(
            first.title,
            "Sublime Gracia- Amazing Grace en español con letra"
        );
        assert_eq!(
            first.subtitle.as_deref(),
            Some("Cinco Solas por Gracia · 6:22")
        );
        assert_eq!(first.meta["provider"], "youtube");
        assert_eq!(first.meta["youtube_id"], "Hc-8A7NA-Uk");
        assert_eq!(
            first.location.as_url(),
            Some("https://www.youtube.com/watch?v=Hc-8A7NA-Uk")
        );
    }

    #[test]
    fn a_stream_says_live_and_missing_fields_keep_the_defaults() {
        let found = source()
            .parse(r#"{"entries": [{"id": "abcdefghijk", "live_status": "is_live"}]}"#)
            .unwrap();
        assert_eq!(found[0].title, "YouTube video abcdefghijk");
        assert_eq!(found[0].subtitle.as_deref(), Some("live"));
    }

    #[test]
    fn lengths_read_like_a_clock() {
        assert_eq!(clock(185), "3:05");
        assert_eq!(clock(3723), "1:02:03");
    }

    #[test]
    fn a_missing_program_is_unavailable() {
        let source = source().program("lantern-no-such-yt-dlp");
        let err = source.search(&Query::new("grace")).unwrap_err();
        assert!(err.to_string().contains("isn't installed"), "{err}");
    }
}
