use std::sync::Arc;

use chorquest_resource::{
    Capabilities, Kind, Location, Query, Resource, ResourceId, Source, SourceError, SourceId,
};

use crate::{Bible, Reference};

/// Serves one [`Bible`] translation. Register one source per translation.
///
/// Resources are [`Kind::Text`] with the text inline, one slide per verse:
///
/// - **Verses** (indexed): id `<source>:John.3.16`, title `Juan 3:16`.
/// - **Passages** (live, from a reference query): id `<source>:John.3.16-18`,
///   title `Juan 3:16-18`.
///
/// Subtitle is the translation's full name, and the abbreviation is a tag,
/// so `juan 3 16 rv1909` finds it. Meta keys: `provider = "bible"`,
/// `translation` (abbreviation), `reference` (OSIS form), and `verses` (the
/// verse number of each slide, comma-separated, so a UI can label slides).
#[derive(Debug, Clone)]
pub struct BibleSource {
    id: SourceId,
    bible: Arc<Bible>,
    index_verses: bool,
}

impl BibleSource {
    /// A source for `bible`, indexing every verse.
    pub fn new(id: SourceId, bible: Bible) -> Self {
        Self {
            id,
            bible: Arc::new(bible),
            index_verses: true,
        }
    }

    /// Whether to index individual verses for text search (default: yes).
    /// Turn it off to answer only reference queries.
    pub fn index_verses(mut self, index: bool) -> Self {
        self.index_verses = index;
        self
    }

    /// The translation being served.
    pub fn bible(&self) -> &Bible {
        &self.bible
    }

    /// The passage for a reference, or `None` if this translation has none of it.
    pub fn passage(&self, reference: &Reference) -> Option<Resource> {
        let verses = self.bible.passage(reference);
        if verses.is_empty() {
            return None;
        }
        let numbers: Vec<String> = verses.iter().map(|(v, _)| v.to_string()).collect();
        let body: Vec<&str> = verses.iter().map(|(_, text)| *text).collect();
        Some(self.resource(reference, body.join("\n\n"), numbers.join(",")))
    }

    fn resource(&self, reference: &Reference, body: String, verses: String) -> Resource {
        let translation = self.bible.translation();
        Resource::new(
            ResourceId::new(&self.id, &reference.osis()),
            reference.display(translation.language),
            Kind::Text,
            Location::Inline(body),
        )
        .with_subtitle(translation.name.clone())
        .with_tag(translation.abbreviation.clone())
        .with_meta("provider", "bible")
        .with_meta("translation", translation.abbreviation.clone())
        .with_meta("reference", reference.osis())
        .with_meta("verses", verses)
    }
}

impl Source for BibleSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn name(&self) -> &str {
        &self.bible.translation().name
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            scan: self.index_verses,
            search: true,
        }
    }

    fn scan(&self) -> Result<Vec<Resource>, SourceError> {
        Ok(self
            .bible
            .verses()
            .map(|(book, chapter, verse, text)| {
                let reference = Reference {
                    book,
                    chapter,
                    verses: vec![(verse, verse)],
                };
                self.resource(&reference, text.to_owned(), verse.to_string())
            })
            .collect())
    }

    fn search(&self, query: &Query) -> Result<Vec<Resource>, SourceError> {
        Ok(Reference::parse_many(&query.text)
            .iter()
            .filter_map(|r| self.passage(r))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Language, Translation};

    fn source() -> BibleSource {
        let translation = Translation::new("RV1909", "Reina-Valera 1909", Language::Spanish);
        let bible = Bible::from_tsv(translation, include_str!("../fixtures/sample.tsv")).unwrap();
        BibleSource::new(SourceId::new("bible-rv1909").unwrap(), bible)
    }

    #[test]
    fn scan_yields_one_resource_per_verse() {
        let items = source().scan().unwrap();
        assert_eq!(items.len(), 8);
        let verse = items.iter().find(|r| r.title == "Juan 3:16").unwrap();
        assert_eq!(verse.id.as_str(), "bible-rv1909:John.3.16");
        assert_eq!(verse.kind, Kind::Text);
        assert_eq!(verse.subtitle.as_deref(), Some("Reina-Valera 1909"));
        assert_eq!(verse.tags, ["RV1909"]);
        assert!(
            verse
                .location
                .as_text()
                .unwrap()
                .starts_with("Porque de tal manera")
        );
    }

    #[test]
    fn reference_queries_return_passages_with_a_slide_per_verse() {
        let found = source().search(&Query::new("sal 23:1-2; jn 3 16")).unwrap();
        assert_eq!(found.len(), 2);

        let psalm = &found[0];
        assert_eq!(psalm.title, "Salmos 23:1-2");
        assert_eq!(psalm.meta["verses"], "1,2");
        assert_eq!(psalm.meta["reference"], "Ps.23.1-2");
        let slides: Vec<_> = psalm.location.as_text().unwrap().split("\n\n").collect();
        assert_eq!(
            slides,
            [
                "Jehová es mi pastor; nada me faltará.",
                "En lugares de delicados pastos me hará yacer: Junto á aguas de reposo me pastoreará."
            ]
        );

        assert_eq!(found[1].title, "Juan 3:16");
    }

    #[test]
    fn non_references_and_missing_passages_return_nothing() {
        assert!(source().search(&Query::new("aleluya")).unwrap().is_empty());
        assert!(
            source()
                .search(&Query::new("Génesis 1:1"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn verse_indexing_can_be_disabled() {
        let source = source().index_verses(false);
        assert_eq!(source.capabilities(), Capabilities::LIVE);
    }
}
