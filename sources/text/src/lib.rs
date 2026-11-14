//! Text typed to be shown: `!x Bienvenidos` puts "Bienvenidos" on stage.
//!
//! What's typed after the bang is the text. ` | ` starts a new line and
//! ` || ` a new slide, since the search box has one line. It answers only
//! behind its bang: anywhere else, typing is a search.

use lantern_resource::{
    Capabilities, Kind, Location, Query, Resource, ResourceId, Source, SourceError, SourceId,
};

/// The source of typed text. Its bang is `x`.
pub struct TextSource {
    id: SourceId,
}

impl TextSource {
    /// A text source with this id.
    pub fn new(id: SourceId) -> Self {
        Self { id }
    }
}

/// What `typed` shows: its lines and slides, trimmed. Slides are set
/// apart by a blank line, as every text resource has them.
pub fn body(typed: &str) -> String {
    let slides: Vec<String> = typed
        .split("||")
        .map(|slide| {
            let lines: Vec<&str> = slide
                .split('|')
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            lines.join("\n")
        })
        .filter(|slide| !slide.is_empty())
        .collect();
    slides.join("\n\n")
}

impl Source for TextSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn name(&self) -> &str {
        "Text"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::LIVE
    }

    fn bangs(&self) -> Vec<&str> {
        vec!["x"]
    }

    fn search(&self, query: &Query) -> Result<Vec<Resource>, SourceError> {
        let body = body(&query.text);
        if !query.sources.contains(&self.id) || body.is_empty() {
            return Ok(Vec::new());
        }
        // Named by how it starts, on one line.
        let title = body.split_whitespace().collect::<Vec<_>>().join(" ");
        let slides = body.split("\n\n").count();
        let about = match slides {
            1 => "your text".to_string(),
            n => format!("your text · {n} slides"),
        };
        Ok(vec![
            Resource::new(
                ResourceId::new(&self.id, &body),
                title,
                Kind::Text,
                Location::Inline(body),
            )
            .with_subtitle(about),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_typed_after_the_bang_is_the_text() {
        let source = TextSource::new(SourceId::new("text").unwrap());
        let ask = |text: &str| {
            let query = Query::new(text).with_source(source.id().clone());
            source.search(&query).unwrap()
        };
        let found = ask("  Bienvenidos a la iglesia ");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Bienvenidos a la iglesia");
        assert_eq!(
            found[0].location.as_text(),
            Some("Bienvenidos a la iglesia")
        );
        assert_eq!(found[0].kind, Kind::Text);
        // A bar is a new line, two a new slide.
        let two = ask("Feliz sábado | a todos || Dios les bendiga");
        assert_eq!(
            two[0].location.as_text(),
            Some("Feliz sábado\na todos\n\nDios les bendiga")
        );
        assert_eq!(two[0].title, "Feliz sábado a todos Dios les bendiga");
        assert_eq!(two[0].subtitle.as_deref(), Some("your text · 2 slides"));
        // The same text is the same thing; nothing typed is nothing.
        assert_eq!(ask("Bienvenidos a la iglesia")[0].id, found[0].id);
        assert!(ask("  ").is_empty());
        assert!(ask(" | || ").is_empty());
        // Without the bang, typing is a search, not a text.
        assert!(
            source
                .search(&Query::new("Bienvenidos"))
                .unwrap()
                .is_empty()
        );
    }
}
