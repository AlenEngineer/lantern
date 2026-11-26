//! YouTube search as a lantern [`Source`], answering `!yt`.
//!
//! It searches one of two ways, chosen by what the operator set up:
//!
//! - With a YouTube Data API key ([`YoutubeSource::with_key`]), through the
//!   API, the way YouTube's terms allow. Each search spends some of the
//!   key's daily quota (a search costs 100 of the 10,000 units a day a key
//!   gets by default, so about a hundred searches).
//! - Else, when the operator chose yt-dlp ([`YoutubeSource::with_yt_dlp`]),
//!   through yt-dlp, which must be installed. That is against YouTube's
//!   terms, so it is only ever done when chosen.
//!
//! With neither, a search fails with a message saying how to set one up.
//! Either way a search takes a moment, so the source is
//! [on request](Capabilities::on_request): only `!yt amazing grace` asks
//! it, never a plain search.
//!
//! Each result is the same resource a pasted YouTube link becomes
//! ([`lantern_source_link::youtube_video`]), titled with the video's title.
//! The subtitle has the channel and the length, or `live` for a stream.
//! The length is also in [`Resource::DURATION`], for timing plans. Results
//! from the API carry [`FETCHED`]: YouTube's terms say what came from the
//! API is fetched again or dropped within 30 days.

pub mod key;

use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use lantern_resource::{Capabilities, Query, Resource, Source, SourceError, SourceId};
use lantern_source_link::youtube_video;
use serde::Deserialize;

/// How many videos a search asks for unless told otherwise.
pub const DEFAULT_RESULTS: usize = 10;

/// Meta: when the title and channel were fetched, in seconds since 1970.
pub const FETCHED: &str = "fetched";

const SEARCH: &str = "https://www.googleapis.com/youtube/v3/search";
const VIDEOS: &str = "https://www.googleapis.com/youtube/v3/videos";

/// Makes the command that runs yt-dlp: the app knows where it is.
type YtDlp = Arc<dyn Fn() -> Command + Send + Sync>;

/// Searches YouTube through its Data API, or yt-dlp when chosen.
#[derive(Clone)]
pub struct YoutubeSource {
    id: SourceId,
    key: Option<String>,
    yt_dlp: Option<YtDlp>,
    results: usize,
}

impl std::fmt::Debug for YoutubeSource {
    /// Leaves the key out, so it never ends up in a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YoutubeSource")
            .field("id", &self.id)
            .field("key", &self.key.as_ref().map(|_| "…"))
            .field("yt_dlp", &self.yt_dlp.is_some())
            .field("results", &self.results)
            .finish()
    }
}

impl YoutubeSource {
    /// A source with neither a key nor yt-dlp: every search says how to
    /// set one up.
    pub fn new(id: SourceId) -> Self {
        Self {
            id,
            key: None,
            yt_dlp: None,
            results: DEFAULT_RESULTS,
        }
    }

    /// Searches through the Data API with `key`. A blank key is none.
    pub fn with_key(mut self, key: Option<String>) -> Self {
        self.key = key.filter(|key| !key.trim().is_empty());
        self
    }

    /// Lets it search through yt-dlp, run by `command`, when it has no key
    /// or the API fails. Only for an operator who chose yt-dlp.
    pub fn with_yt_dlp(mut self, command: impl Fn() -> Command + Send + Sync + 'static) -> Self {
        self.yt_dlp = Some(Arc::new(command));
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

    /// Turns the Data API's answers into resources, in the search's order:
    /// `search` from `search.list`, and `videos` from `videos.list` for the
    /// same ids, which has their lengths (`None` if that call failed).
    /// `fetched` is when, in seconds since 1970.
    pub fn parse_api(
        &self,
        search: &str,
        videos: Option<&str>,
        fetched: u64,
    ) -> Result<Vec<Resource>, SourceError> {
        let found: ApiSearch =
            serde_json::from_str(search).map_err(|err| SourceError::Other(Box::new(err)))?;
        let lengths: Vec<ApiVideo> = videos
            .and_then(|json| serde_json::from_str::<ApiVideos>(json).ok())
            .map(|videos| videos.items)
            .unwrap_or_default();
        Ok(found
            .items
            .into_iter()
            .filter_map(|item| {
                let id = item.id.video_id?;
                let live = item.snippet.live_broadcast_content.as_deref() == Some("live");
                let duration = lengths
                    .iter()
                    .find(|video| video.id == id)
                    .and_then(|video| iso_seconds(&video.content_details.duration))
                    .filter(|seconds| *seconds > 0.0);
                let entry = Entry {
                    id,
                    title: item.snippet.title.map(|t| unescape(&t)),
                    duration,
                    channel: item.snippet.channel_title.map(|t| unescape(&t)),
                    uploader: None,
                    live_status: live.then(|| "is_live".to_string()),
                };
                Some(self.resource(entry).with_meta(FETCHED, fetched.to_string()))
            })
            .collect())
    }

    fn resource(&self, entry: Entry) -> Resource {
        let mut resource = youtube_video(&self.id, &entry.id);
        let live = entry.live_status.as_deref() == Some("is_live");
        let length = if live {
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
        if let (Some(seconds), false) = (entry.duration, live) {
            resource = resource.with_meta(Resource::DURATION, (seconds as u64).to_string());
        }
        resource
    }

    /// A search through the Data API.
    fn search_api(
        &self,
        key: &str,
        text: &str,
        wanted: usize,
    ) -> Result<Vec<Resource>, SourceError> {
        let count = wanted.clamp(1, 50).to_string();
        let answer = key::get(
            SEARCH,
            &[
                ("part", "snippet"),
                ("type", "video"),
                ("safeSearch", "strict"),
                ("maxResults", &count),
                ("q", text),
                ("key", key),
            ],
        );
        // What went wrong, in the same words a key check uses.
        let search = match answer {
            Ok((status, body)) if (200..300).contains(&status) => body,
            Ok((status, body)) => {
                let check = key::read_answer(status, &body);
                return Err(SourceError::Unavailable(format!(
                    "YouTube Data API: {}",
                    check.says()
                )));
            }
            Err(_) => {
                return Err(SourceError::Unavailable(
                    "YouTube Data API: Can't reach YouTube".into(),
                ));
            }
        };
        let ids: Vec<String> = serde_json::from_str::<ApiSearch>(&search)
            .map(|found| {
                found
                    .items
                    .into_iter()
                    .filter_map(|i| i.id.video_id)
                    .collect()
            })
            .unwrap_or_default();
        // The lengths are a second call. Without them the results still do.
        let videos = (!ids.is_empty())
            .then(|| {
                ureq::get(VIDEOS)
                    .query("part", "contentDetails")
                    .query("id", ids.join(","))
                    .query("key", key)
                    .call()
                    .and_then(|mut r| r.body_mut().read_to_string())
                    .ok()
            })
            .flatten();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.parse_api(&search, videos.as_deref(), now)
    }

    /// A search through yt-dlp.
    fn search_yt_dlp(
        &self,
        yt_dlp: &YtDlp,
        text: &str,
        wanted: usize,
    ) -> Result<Vec<Resource>, SourceError> {
        let output = yt_dlp()
            .args(["--flat-playlist", "--dump-single-json", "--no-warnings"])
            .arg(format!("ytsearch{wanted}:{text}"))
            .output()
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => {
                    SourceError::Unavailable("yt-dlp isn't installed".into())
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

/// `search.list`'s answer.
#[derive(Deserialize)]
struct ApiSearch {
    #[serde(default)]
    items: Vec<ApiItem>,
}

#[derive(Deserialize)]
struct ApiItem {
    id: ApiId,
    snippet: ApiSnippet,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiId {
    video_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiSnippet {
    title: Option<String>,
    channel_title: Option<String>,
    /// `none`, `live` or `upcoming`.
    live_broadcast_content: Option<String>,
}

/// `videos.list`'s answer with `part=contentDetails`.
#[derive(Deserialize)]
struct ApiVideos {
    #[serde(default)]
    items: Vec<ApiVideo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiVideo {
    id: String,
    content_details: ApiDetails,
}

#[derive(Deserialize)]
struct ApiDetails {
    /// ISO 8601: `PT6M22S`, `PT1H2M3S`, `P0D` for a live stream.
    #[serde(default)]
    duration: String,
}

/// Seconds in an ISO 8601 duration such as `PT1H2M3S` or `P1DT2H`.
fn iso_seconds(text: &str) -> Option<f64> {
    let rest = text.strip_prefix('P')?;
    let mut seconds = 0.0;
    let mut number = String::new();
    let mut time = false;
    for c in rest.chars() {
        match c {
            'T' => time = true,
            '0'..='9' | '.' => number.push(c),
            unit => {
                let n: f64 = number.parse().ok()?;
                number.clear();
                seconds += n * match (unit, time) {
                    ('W', false) => 604_800.0,
                    ('D', false) => 86_400.0,
                    ('H', true) => 3_600.0,
                    ('M', true) => 60.0,
                    ('S', true) => 1.0,
                    _ => return None,
                };
            }
        }
    }
    number.is_empty().then_some(seconds)
}

/// The API writes titles as HTML: `&amp;`, `&#39;`, `&quot;`.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|n| n.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
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

    fn bangs(&self) -> Vec<&str> {
        vec!["yt"]
    }

    fn search(&self, query: &Query) -> Result<Vec<Resource>, SourceError> {
        let text = query.text.trim();
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let wanted = self.results.min(query.limit);
        let api = self
            .key
            .as_deref()
            .map(|key| self.search_api(key, text, wanted));
        match (api, &self.yt_dlp) {
            (Some(Ok(found)), _) => Ok(found),
            // The API failed (quota spent, offline): yt-dlp, if chosen.
            (Some(Err(_)), Some(yt_dlp)) => self.search_yt_dlp(yt_dlp, text, wanted),
            (Some(Err(err)), None) => Err(err),
            (None, Some(yt_dlp)) => self.search_yt_dlp(yt_dlp, text, wanted),
            (None, None) => Err(SourceError::Unavailable(key::NEEDS_SETUP.into())),
        }
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
        assert_eq!(first.meta[Resource::DURATION], "382");
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
        assert!(!found[0].meta.contains_key(Resource::DURATION));
    }

    #[test]
    fn lengths_read_like_a_clock() {
        assert_eq!(clock(185), "3:05");
        assert_eq!(clock(3723), "1:02:03");
    }

    #[test]
    fn a_missing_program_is_unavailable() {
        let source = source().with_yt_dlp(|| Command::new("lantern-no-such-yt-dlp"));
        let err = source.search(&Query::new("grace")).unwrap_err();
        assert!(err.to_string().contains("isn't installed"), "{err}");
    }

    #[test]
    fn with_neither_a_key_nor_yt_dlp_a_search_says_how_to_set_one_up() {
        let err = source().search(&Query::new("grace")).unwrap_err();
        assert!(err.to_string().contains("Settings → YouTube"), "{err}");
    }

    #[test]
    fn api_results_have_titles_channels_lengths_and_when_they_were_fetched() {
        let found = source()
            .with_key(Some("key".into()))
            .parse_api(
                include_str!("../fixtures/api-search.json"),
                Some(include_str!("../fixtures/api-videos.json")),
                1_790_000_000,
            )
            .unwrap();
        assert_eq!(found.len(), 3, "the channel result has no video id");
        let first = &found[0];
        assert_eq!(
            first.title,
            "Sublime Gracia - Amazing Grace en español con letra"
        );
        assert_eq!(
            first.subtitle.as_deref(),
            Some("Cinco Solas por Gracia · 6:22")
        );
        assert_eq!(first.meta["youtube_id"], "Hc-8A7NA-Uk");
        assert_eq!(first.meta[Resource::DURATION], "382");
        assert_eq!(first.meta[FETCHED], "1790000000");
        assert_eq!(found[1].title, "Himno \"Sublime Gracia\" & más");
        assert_eq!(
            found[1].subtitle.as_deref(),
            Some("Himnos de Fe y Alabanzas · 1:02:03")
        );
        assert_eq!(found[2].subtitle.as_deref(), Some("Iglesia en vivo · live"));
        assert!(!found[2].meta.contains_key(Resource::DURATION));
    }

    #[test]
    fn api_results_without_lengths_still_do() {
        let found = source()
            .parse_api(include_str!("../fixtures/api-search.json"), None, 0)
            .unwrap();
        assert_eq!(found[0].subtitle.as_deref(), Some("Cinco Solas por Gracia"));
    }

    #[test]
    fn iso_lengths_read_as_seconds() {
        assert_eq!(iso_seconds("PT6M22S"), Some(382.0));
        assert_eq!(iso_seconds("PT1H2M3S"), Some(3723.0));
        assert_eq!(iso_seconds("P1DT1S"), Some(86_401.0));
        assert_eq!(iso_seconds("P0D"), Some(0.0));
        assert_eq!(iso_seconds("6:22"), None);
        assert_eq!(iso_seconds("PT6"), None);
    }

    #[test]
    fn titles_lose_their_html_escapes() {
        assert_eq!(
            unescape("Rock &amp; Roll &#39;n&#x27; &quot;x&quot;"),
            "Rock & Roll 'n' \"x\""
        );
        assert_eq!(unescape("a & b &bogus; &"), "a & b &bogus; &");
    }

    #[test]
    fn the_key_stays_out_of_debug_output() {
        let source = source().with_key(Some("secret-key".into()));
        assert!(!format!("{source:?}").contains("secret-key"));
    }
}
