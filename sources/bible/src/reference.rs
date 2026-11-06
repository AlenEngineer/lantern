use std::fmt::Write as _;

use crate::{Book, Language};

/// A passage within one chapter: `Juan 3:16-18,20`, or a whole chapter (`Salmos 23`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// The book.
    pub book: Book,
    /// Chapter, from 1.
    pub chapter: u16,
    /// Inclusive verse ranges, in the order written. Empty means the whole chapter.
    pub verses: Vec<(u16, u16)>,
}

impl Reference {
    /// Parses a reference as people type it:
    ///
    /// - `Juan 3:16`, `Juan 3.16`, `Juan 3,16`, `jn 3 16`, `juan3:16`
    /// - `Juan 3:16-18`, `Juan 3:16,18,20-22`
    /// - `Salmos 23`, `Sal 23:` (whole chapter, including while still typing)
    /// - `1 Co 13:4-7`, `primera de Juan 4:8`, `San Juan 14:6`
    ///
    /// Returns `None` for anything else, including chapter or verse 0 and
    /// backwards ranges. Whether the passage exists depends on the translation.
    pub fn parse(input: &str) -> Option<Reference> {
        let text = input.trim();
        // The book is everything before the first digit that follows a letter;
        // a leading digit is an ordinal ("1 Co").
        let mut seen_letter = false;
        let split = text.char_indices().find_map(|(i, c)| {
            if c.is_alphabetic() {
                seen_letter = true;
                None
            } else if c.is_ascii_digit() && seen_letter {
                Some(i)
            } else {
                None
            }
        })?;
        let (book_part, numbers) = text.split_at(split);
        let book = Book::find(book_part)?;
        let (chapter, verses) = parse_numbers(numbers)?;
        Some(Reference {
            book,
            chapter,
            verses,
        })
    }

    /// Parses several references separated by `;`, skipping the ones that don't parse.
    pub fn parse_many(input: &str) -> Vec<Reference> {
        input.split(';').filter_map(Reference::parse).collect()
    }

    /// Whether this covers the whole chapter.
    pub fn is_whole_chapter(&self) -> bool {
        self.verses.is_empty()
    }

    /// Human form in `language`: `"Juan 3:16-18"`, `"Salmos 23"`.
    pub fn display(&self, language: Language) -> String {
        self.format(self.book.name(language), ":")
    }

    /// Machine form, used in resource ids: `"John.3.16-18"`, `"Ps.23"`.
    pub fn osis(&self) -> String {
        self.format(self.book.osis(), ".")
    }

    fn format(&self, book: &str, separator: &str) -> String {
        let book_separator = if separator == "." { "." } else { " " };
        let mut out = format!("{book}{book_separator}{}", self.chapter);
        for (i, (start, end)) in self.verses.iter().enumerate() {
            out.push_str(if i == 0 { separator } else { "," });
            let _ = write!(out, "{start}");
            if end != start {
                let _ = write!(out, "-{end}");
            }
        }
        out
    }
}

/// `"3:16-18,20"` → `(3, [(16, 18), (20, 20)])`.
fn parse_numbers(text: &str) -> Option<(u16, Vec<(u16, u16)>)> {
    let text = text.trim().trim_end_matches(|c: char| {
        c.is_whitespace() || matches!(c, ':' | '.' | ',' | '-' | '–' | '—')
    });

    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let chapter: u16 = text[..digits].parse().ok().filter(|&c| c > 0)?;
    let rest = &text[digits..];
    if rest.is_empty() {
        return Some((chapter, Vec::new()));
    }

    let trimmed = rest.trim_start();
    let verses_text = match trimmed.strip_prefix([':', '.', ',']) {
        Some(after) => after,
        // "juan 3 16": a space alone also separates chapter and verse.
        None if trimmed.len() < rest.len() => trimmed,
        None => return None,
    };

    let mut verses = Vec::new();
    for item in verses_text.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let (start, end) = match item.split_once(['-', '–', '—']) {
            Some((a, b)) => (a.trim().parse().ok()?, b.trim().parse().ok()?),
            None => {
                let verse = item.parse().ok()?;
                (verse, verse)
            }
        };
        if start == 0 || end < start {
            return None;
        }
        verses.push((start, end));
    }
    Some((chapter, verses))
}

#[cfg(test)]
mod tests {
    use super::*;

    type Parsed = (String, u16, Vec<(u16, u16)>);

    fn parse(text: &str) -> Option<Parsed> {
        Reference::parse(text).map(|r| (r.book.osis().to_owned(), r.chapter, r.verses))
    }

    fn john(chapter: u16, verses: &[(u16, u16)]) -> Option<Parsed> {
        Some(("John".to_owned(), chapter, verses.to_vec()))
    }

    #[test]
    fn many_ways_to_write_john_3_16() {
        for text in [
            "Juan 3:16",
            "juan 3.16",
            "Juan 3,16",
            "jn 3 16",
            "juan3:16",
            "San Juan 3:16",
            " John 3 : 16 ",
        ] {
            assert_eq!(parse(text), john(3, &[(16, 16)]), "{text}");
        }
    }

    #[test]
    fn ranges_and_lists() {
        assert_eq!(parse("Juan 3:16-18"), john(3, &[(16, 18)]));
        assert_eq!(parse("Juan 3:16–18"), john(3, &[(16, 18)]));
        assert_eq!(
            parse("Juan 3:16, 18, 20-22"),
            john(3, &[(16, 16), (18, 18), (20, 22)])
        );
    }

    #[test]
    fn whole_chapters_and_partial_typing() {
        assert_eq!(parse("Sal 23"), Some(("Ps".into(), 23, vec![])));
        assert_eq!(parse("Juan 3:"), john(3, &[]));
        assert_eq!(parse("Juan 3:16-"), john(3, &[(16, 16)]));
    }

    #[test]
    fn numbered_books() {
        assert_eq!(
            parse("1 Co 13:4-7"),
            Some(("1Cor".into(), 13, vec![(4, 7)]))
        );
        assert_eq!(parse("1co13:4"), Some(("1Cor".into(), 13, vec![(4, 4)])));
        assert_eq!(
            parse("primera de juan 4:8"),
            Some(("1John".into(), 4, vec![(8, 8)]))
        );
    }

    #[test]
    fn rejects_nonsense() {
        for text in [
            "",
            "Juan",
            "3:16",
            "Juan 0",
            "Juan 3:0",
            "Juan 3:18-16",
            "Juan 3a",
            "cancion 3",
            "ju 3:16",
        ] {
            assert_eq!(parse(text), None, "{text}");
        }
    }

    #[test]
    fn several_references() {
        let refs = Reference::parse_many("Juan 3:16; Ro 5:8; nonsense");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[1].osis(), "Rom.5.8");
    }

    #[test]
    fn display_forms() {
        let r = Reference::parse("1 co 13:4-7,13").unwrap();
        assert_eq!(r.display(Language::Spanish), "1 Corintios 13:4-7,13");
        assert_eq!(r.display(Language::English), "1 Corinthians 13:4-7,13");
        assert_eq!(r.osis(), "1Cor.13.4-7,13");
        assert_eq!(
            Reference::parse("sal 23")
                .unwrap()
                .display(Language::Spanish),
            "Salmos 23"
        );
    }
}
