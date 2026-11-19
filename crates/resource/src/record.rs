use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{Kind, Location, ResourceId};

/// Anything that can be put on stage.
///
/// A resource is a self-contained *snapshot*: it carries everything needed to
/// present it, so a saved plan keeps working even if the source that produced
/// it is gone or re-indexed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    /// Stable, globally unique id.
    pub id: ResourceId,
    /// Main display name, and the most important field for search.
    pub title: String,
    /// Secondary line: folder, channel, Bible version...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// How to present it.
    pub kind: Kind,
    /// Where its content lives.
    pub location: Location,
    /// Extra searchable words (folder names, categories, authors...).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Free-form, source-specific data that is not searched (size, duration,
    /// YouTube id, start time...). Sources should document their keys.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meta: BTreeMap<String, String>,
}

impl Resource {
    /// Meta key for the URL of a smaller copy of an image at a
    /// [`Location::Url`]: the stage shows it when the full image can't be
    /// fetched, and the dashboard can use it as a preview.
    pub const THUMBNAIL: &'static str = "thumbnail";

    /// Meta key for how long the resource takes to present, in whole
    /// seconds: a video's length, or a hymn's from its score. Plans use it
    /// to time their sections.
    pub const DURATION: &'static str = "duration_seconds";

    /// Meta key for which page of a paged file (a PDF, a deck) the
    /// resource is, counted from 0: the stage shows that page alone.
    pub const PAGE: &'static str = "page";

    /// Page `n` of this paged file, counted from 0, as a resource of its
    /// own: the same file, its own id, and a title that says which page.
    pub fn page(&self, n: usize) -> Resource {
        let mut page = self.clone();
        page.id = self.id.part(&format!("page{}", n + 1));
        page.title = format!("{} · {}", self.title, n + 1);
        page.meta.insert(Self::PAGE.to_string(), n.to_string());
        page
    }

    /// The whole file this page was opened out of, if it is a page: the
    /// resource [`page`](Self::page) was given.
    pub fn file_of_page(&self) -> Option<Resource> {
        let n = self.page_of()?;
        let id = String::from(self.id.clone());
        let id = id.strip_suffix(&format!("#page{}", n + 1))?.to_string();
        let mut file = self.clone();
        file.id = ResourceId::try_from(id).ok()?;
        if let Some(title) = self.title.strip_suffix(&format!(" · {}", n + 1)) {
            file.title = title.to_string();
        }
        file.meta.remove(Self::PAGE);
        Some(file)
    }

    /// Which page of its file this is, counted from 0, if it is one.
    pub fn page_of(&self) -> Option<usize> {
        self.meta.get(Self::PAGE)?.parse().ok()
    }

    /// A resource with the required fields; use the `with_*` methods for the rest.
    pub fn new(id: ResourceId, title: impl Into<String>, kind: Kind, location: Location) -> Self {
        Self {
            id,
            title: title.into(),
            subtitle: None,
            kind,
            location,
            tags: Vec::new(),
            meta: BTreeMap::new(),
        }
    }

    /// Sets the subtitle.
    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Adds a tag.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Sets a metadata entry.
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.meta.insert(key.into(), value.into());
        self
    }
}

/// A search result.
#[derive(Debug, Clone)]
pub struct Hit {
    /// The matching resource (shared, cheap to clone).
    pub resource: Arc<Resource>,
    /// Relevance; higher is better. Only comparable within the same origin.
    pub score: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceId;

    #[test]
    fn a_page_knows_the_file_it_was_opened_out_of() {
        let source = SourceId::new("local").unwrap();
        let deck = Resource::new(
            ResourceId::new(&source, "/decks/news.pptx"),
            "News",
            Kind::Presentation,
            Location::File("/decks/news.pptx".into()),
        )
        .with_meta("modified", "1");
        let page = deck.page(11);
        assert_eq!(page.file_of_page(), Some(deck.clone()));
        assert_eq!(deck.file_of_page(), None, "a whole file is no page");
    }

    #[test]
    fn serde_round_trip() {
        let source = SourceId::new("local").unwrap();
        let resource = Resource::new(
            ResourceId::new(&source, "/music/a.mp3"),
            "Santo, santo, santo",
            Kind::Audio,
            Location::File("/music/a.mp3".into()),
        )
        .with_subtitle("Music")
        .with_tag("hymns")
        .with_meta("size", "1024");

        let json = serde_json::to_string(&resource).unwrap();
        let back: Resource = serde_json::from_str(&json).unwrap();
        assert_eq!(resource, back);
    }
}
