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
