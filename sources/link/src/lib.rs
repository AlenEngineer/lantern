//! Pasted links as a lantern [`Source`].
//!
//! The fastest way to get something from the internet on stage is to paste
//! its link into the search box. This live source recognizes a URL in the
//! query and turns it into a [`Resource`] without touching the network:
//!
//! - YouTube links (`watch`, `youtu.be`, `shorts`, `live`, `embed`) become a
//!   [`Kind::Video`] with a canonical URL. Meta: `provider = "youtube"`,
//!   `youtube_id`, and `start_seconds` if the link had `t=`/`start=`.
//! - Links to files with a known extension (`.jpg`, `.mp4`, `.pdf`...) get
//!   that kind.
//! - Anything else is a [`Kind::Web`] page.
//!
//! Anything that doesn't look like a link returns no results, so this source
//! is cheap to leave enabled.

use std::path::Path;

use lantern_resource::{
    Capabilities, Kind, Location, Query, Resource, ResourceId, Source, SourceError, SourceId,
};
use url::Url;

/// Recognizes links typed or pasted into the search box.
#[derive(Debug, Clone)]
pub struct LinkSource {
    id: SourceId,
}

impl LinkSource {
    /// A link source with the given id.
    pub fn new(id: SourceId) -> Self {
        Self { id }
    }

    /// Turns `text` into a resource, if it is a link.
    pub fn resolve(&self, text: &str) -> Option<Resource> {
        let url = parse(text)?;

        if let Some(video) = youtube_id(&url) {
            let mut resource = youtube_video(&self.id, &video);
            let start = url
                .query_pairs()
                .find(|(k, _)| k == "t" || k == "start")
                .and_then(|(_, v)| parse_seconds(&v));
            if let Some(start) = start {
                resource = resource.with_meta("start_seconds", start.to_string());
            }
            return Some(resource);
        }

        let host = url.host_str().unwrap_or_default().to_owned();
        let kind = Path::new(url.path())
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Kind::from_extension)
            .unwrap_or(Kind::Web);
        let title = url
            .path_segments()
            .and_then(|mut segments| segments.rfind(|s| !s.is_empty()))
            .map(|s| s.replace("%20", " "))
            .unwrap_or_else(|| host.clone());
        Some(
            Resource::new(
                ResourceId::new(&self.id, url.as_str()),
                title,
                kind,
                Location::Url(url.to_string()),
            )
            .with_subtitle(host),
        )
    }
}

impl Source for LinkSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn name(&self) -> &str {
        "Links"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::LIVE
    }

    fn search(&self, query: &Query) -> Result<Vec<Resource>, SourceError> {
        Ok(self.resolve(&query.text).into_iter().collect())
    }
}

/// A YouTube video as a resource: a [`Kind::Video`] at its canonical watch
/// URL, titled `YouTube video <id>`, with meta `provider = "youtube"` and
/// `youtube_id`. Every source that finds YouTube videos builds them here, so
/// the app downloads and plays them alike.
pub fn youtube_video(source: &SourceId, video: &str) -> Resource {
    let canonical = format!("https://www.youtube.com/watch?v={video}");
    Resource::new(
        ResourceId::new(source, &canonical),
        format!("YouTube video {video}"),
        Kind::Video,
        Location::Url(canonical),
    )
    .with_subtitle("YouTube")
    .with_tag("youtube")
    .with_meta("provider", "youtube")
    .with_meta("youtube_id", video)
}

/// Accepts `http(s)://...`, and a few scheme-less forms people actually paste.
fn parse(text: &str) -> Option<Url> {
    let text = text.trim();
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    let candidate = if lower.starts_with("http://") || lower.starts_with("https://") {
        text.to_owned()
    } else if ["www.", "youtu.be/", "youtube.com/", "m.youtube.com/"]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        format!("https://{text}")
    } else {
        return None;
    };
    Url::parse(&candidate)
        .ok()
        .filter(|u| u.host_str().is_some())
}

/// The 11-character video id of a YouTube link.
pub fn youtube_id(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    let host = host
        .strip_prefix("www.")
        .or_else(|| host.strip_prefix("m."))
        .unwrap_or(host);
    let id = match host {
        "youtu.be" => url.path_segments()?.next()?.to_owned(),
        "youtube.com" | "music.youtube.com" | "youtube-nocookie.com" => {
            let mut segments = url.path_segments()?;
            match segments.next()? {
                "watch" => url.query_pairs().find(|(k, _)| k == "v")?.1.into_owned(),
                "shorts" | "live" | "embed" | "v" => segments.next()?.to_owned(),
                _ => return None,
            }
        }
        _ => return None,
    };
    let valid = id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    valid.then_some(id)
}

/// `"90"`, `"90s"`, `"1m30s"`, `"1h2m3s"` → seconds.
fn parse_seconds(value: &str) -> Option<u64> {
    if let Ok(seconds) = value.parse() {
        return Some(seconds);
    }
    let mut total = 0u64;
    let mut digits = String::new();
    for c in value.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            let n: u64 = digits.parse().ok()?;
            digits.clear();
            let unit = match c {
                'h' => 3600,
                'm' => 60,
                's' => 1,
                _ => return None,
            };
            total = total.checked_add(n.checked_mul(unit)?)?;
        }
    }
    digits.is_empty().then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links() -> LinkSource {
        LinkSource::new(SourceId::new("link").unwrap())
    }

    #[test]
    fn recognizes_youtube_in_all_its_forms() {
        for link in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=x",
            "https://youtu.be/dQw4w9WgXcQ",
            "youtu.be/dQw4w9WgXcQ",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ",
        ] {
            let r = links().resolve(link).unwrap_or_else(|| panic!("{link}"));
            assert_eq!(r.kind, Kind::Video, "{link}");
            assert_eq!(r.meta["youtube_id"], "dQw4w9WgXcQ", "{link}");
            assert_eq!(
                r.location,
                Location::Url("https://www.youtube.com/watch?v=dQw4w9WgXcQ".into())
            );
        }
    }

    #[test]
    fn keeps_youtube_start_time() {
        let r = links()
            .resolve("https://youtu.be/dQw4w9WgXcQ?t=1m30s")
            .unwrap();
        assert_eq!(r.meta["start_seconds"], "90");
        assert_eq!(parse_seconds("75"), Some(75));
        assert_eq!(parse_seconds("1h2m3s"), Some(3723));
        assert_eq!(parse_seconds("3x"), None);
        assert_eq!(parse_seconds("12m5"), None);
    }

    #[test]
    fn files_and_pages() {
        let image = links()
            .resolve("https://example.com/img/sunset.JPG?w=800")
            .unwrap();
        assert_eq!(image.kind, Kind::Image);
        assert_eq!(image.title, "sunset.JPG");
        assert_eq!(image.subtitle.as_deref(), Some("example.com"));

        let page = links().resolve("www.example.com/").unwrap();
        assert_eq!(page.kind, Kind::Web);
        assert_eq!(page.title, "www.example.com");
    }

    #[test]
    fn ignores_non_links() {
        for text in ["", "santo santo", "juan 3:16", "youtube", "https://"] {
            assert!(links().resolve(text).is_none(), "{text}");
        }
        assert!(
            links()
                .resolve("https://www.youtube.com/watch?v=short")
                .unwrap()
                .kind
                == Kind::Web
        );
    }
}
