//! Image search through the [Brave Search API], as a lantern [`Source`]
//! answering `!gi`.
//!
//! Each search is a metered API call, so the source is
//! [on request](Capabilities::on_request): only `!gi sunset cross` asks it.
//! It needs an API key from Brave; without one, every search fails with a
//! message saying so. SafeSearch is always strict.
//!
//! Each result is a [`Kind::Image`] at the original image's URL, titled with
//! the image's title (or its site), with the site and size as subtitle.
//! Meta: `provider = "brave"`, `page` (where the image was found), and
//! [`Resource::THUMBNAIL`] (Brave's resized copy, which loads even when the
//! original site refuses).
//!
//! [Brave Search API]: https://brave.com/search/api/

use lantern_resource::{
    Capabilities, Kind, Location, Query, Resource, ResourceId, Source, SourceError, SourceId,
};
use serde::Deserialize;

const ENDPOINT: &str = "https://api.search.brave.com/res/v1/images/search";

/// How many images a search asks for unless the query wants fewer.
pub const DEFAULT_RESULTS: usize = 20;

/// Searches images with the Brave Search API.
#[derive(Clone)]
pub struct BraveImages {
    id: SourceId,
    key: Option<String>,
}

impl std::fmt::Debug for BraveImages {
    /// Leaves the key out, so it never ends up in a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BraveImages")
            .field("id", &self.id)
            .field("key", &self.key.as_ref().map(|_| "…"))
            .finish()
    }
}

impl BraveImages {
    /// A source using `key`, or one that explains it needs a key.
    pub fn new(id: SourceId, key: Option<String>) -> Self {
        let key = key.filter(|key| !key.trim().is_empty());
        Self { id, key }
    }

    /// Turns the API's JSON answer into resources, in its order. Results
    /// without an image URL are skipped.
    pub fn parse(&self, json: &str) -> Result<Vec<Resource>, SourceError> {
        let found: Found =
            serde_json::from_str(json).map_err(|err| SourceError::Other(Box::new(err)))?;
        Ok(found
            .results
            .into_iter()
            .filter_map(|image| self.resource(image))
            .collect())
    }

    fn resource(&self, image: Image) -> Option<Resource> {
        let url = image.properties.url?;
        let site = image.source.unwrap_or_default();
        let title = image
            .title
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| site.clone());
        let size = match (image.properties.width, image.properties.height) {
            (Some(width), Some(height)) => Some(format!("{width}×{height}")),
            _ => None,
        };
        let subtitle: Vec<String> = Some(site)
            .filter(|s| !s.is_empty())
            .into_iter()
            .chain(size)
            .collect();
        let mut resource = Resource::new(
            ResourceId::new(&self.id, &url),
            title,
            Kind::Image,
            Location::Url(url),
        )
        .with_meta("provider", "brave");
        if !subtitle.is_empty() {
            resource = resource.with_subtitle(subtitle.join(" · "));
        }
        if let Some(page) = image.url {
            resource = resource.with_meta("page", page);
        }
        if let Some(thumbnail) = image.thumbnail.and_then(|t| t.src) {
            resource = resource.with_meta(Resource::THUMBNAIL, thumbnail);
        }
        Some(resource)
    }
}

/// The API's answer, keeping what the resources need.
#[derive(Deserialize)]
struct Found {
    #[serde(default)]
    results: Vec<Image>,
}

#[derive(Deserialize)]
struct Image {
    title: Option<String>,
    /// The page the image was found on.
    url: Option<String>,
    /// The page's domain.
    source: Option<String>,
    thumbnail: Option<Thumbnail>,
    #[serde(default)]
    properties: Properties,
}

#[derive(Deserialize)]
struct Thumbnail {
    src: Option<String>,
}

#[derive(Default, Deserialize)]
struct Properties {
    /// The original image.
    url: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

impl Source for BraveImages {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn name(&self) -> &str {
        "Images (Brave)"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::ON_REQUEST
    }

    fn bangs(&self) -> Vec<&str> {
        vec!["gi"]
    }

    fn search(&self, query: &Query) -> Result<Vec<Resource>, SourceError> {
        let text = query.text.trim();
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let Some(key) = &self.key else {
            return Err(SourceError::Unavailable(
                "image search needs a Brave Search API key: add `brave_api_key` to the config"
                    .into(),
            ));
        };
        let count = DEFAULT_RESULTS.min(query.limit).to_string();
        let body = ureq::get(ENDPOINT)
            .header("Accept", "application/json")
            .header("X-Subscription-Token", key)
            .query("q", text)
            .query("count", &count)
            .query("safesearch", "strict")
            .call()
            .and_then(|mut response| response.body_mut().read_to_string())
            .map_err(|err| SourceError::Unavailable(format!("Brave Search: {err}")))?;
        self.parse(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(key: Option<&str>) -> BraveImages {
        BraveImages::new(SourceId::new("images").unwrap(), key.map(String::from))
    }

    #[test]
    fn results_are_images_at_the_original_url_with_a_thumbnail() {
        let found = source(Some("k"))
            .parse(include_str!("../fixtures/images.json"))
            .unwrap();
        assert_eq!(found.len(), 2);
        let first = &found[0];
        assert_eq!(first.title, "Cruz al atardecer en la montaña");
        assert_eq!(first.kind, Kind::Image);
        assert_eq!(
            first.location.as_url(),
            Some("https://example.org/images/cruz-atardecer.jpg")
        );
        assert_eq!(first.subtitle.as_deref(), Some("example.org · 1920×1280"));
        assert_eq!(
            first.meta["page"],
            "https://example.org/fotos/cruz-atardecer"
        );
        assert!(first.meta[Resource::THUMBNAIL].starts_with("https://imgs.search.brave.com/"));
    }

    #[test]
    fn an_untitled_image_is_named_after_its_site() {
        let found = source(Some("k"))
            .parse(include_str!("../fixtures/images.json"))
            .unwrap();
        assert_eq!(found[1].title, "fotos.example.net");
        assert_eq!(found[1].subtitle.as_deref(), Some("fotos.example.net"));
    }

    #[test]
    fn without_a_key_a_search_says_how_to_add_one() {
        for key in [None, Some("  ")] {
            let err = source(key).search(&Query::new("cross")).unwrap_err();
            assert!(err.to_string().contains("brave_api_key"), "{err}");
        }
    }

    #[test]
    fn the_key_stays_out_of_debug_output() {
        let shown = format!("{:?}", source(Some("secret-key")));
        assert!(!shown.contains("secret-key"), "{shown}");
    }
}
