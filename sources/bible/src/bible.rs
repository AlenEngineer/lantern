use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Book, Language, Reference};

/// Errors loading a translation.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// Reading the file failed.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// The JSON is malformed or doesn't have the expected layout.
    #[error("invalid json: {0}")]
    Json(#[from] serde_json::Error),
    /// The JSON doesn't have exactly 66 books.
    #[error("expected 66 books, found {0}")]
    BookCount(usize),
    /// A TSV line couldn't be understood.
    #[error("line {line}: {reason}")]
    Tsv {
        /// 1-based line number.
        line: usize,
        /// What was wrong.
        reason: String,
    },
    /// The file extension is neither `.json` nor `.tsv`.
    #[error("unknown bible format: {0}")]
    UnknownFormat(PathBuf),
}

/// Describes a translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translation {
    /// Short name shown next to references, e.g. `"RV1909"`. Also searchable.
    pub abbreviation: String,
    /// Full name, e.g. `"Reina-Valera 1909"`.
    pub name: String,
    /// Language of book names in references.
    pub language: Language,
}

impl Translation {
    /// A translation description.
    pub fn new(
        abbreviation: impl Into<String>,
        name: impl Into<String>,
        language: Language,
    ) -> Self {
        Self {
            abbreviation: abbreviation.into(),
            name: name.into(),
            language,
        }
    }
}

/// The full text of one translation, held in memory.
///
/// A complete Bible is about 31,000 verses and a few megabytes, which is
/// small enough to load at startup and look up without any I/O.
#[derive(Debug, Clone)]
pub struct Bible {
    translation: Translation,
    /// `books[book][chapter - 1][verse - 1]`. Missing verses are empty strings.
    books: Vec<Vec<Vec<String>>>,
}

#[derive(Deserialize)]
struct JsonBook {
    chapters: Vec<Vec<String>>,
}

impl Bible {
    /// An empty Bible, to fill with [`set_verse`](Self::set_verse).
    pub fn new(translation: Translation) -> Self {
        Self {
            translation,
            books: vec![Vec::new(); Book::COUNT],
        }
    }

    /// The translation.
    pub fn translation(&self) -> &Translation {
        &self.translation
    }

    /// Sets the text of a verse (chapter and verse from 1). Whitespace is
    /// normalized, so a verse is always a single paragraph.
    pub fn set_verse(&mut self, book: Book, chapter: u16, verse: u16, text: &str) {
        if chapter == 0 || verse == 0 {
            return;
        }
        let chapters = &mut self.books[book.index()];
        let (c, v) = (usize::from(chapter), usize::from(verse));
        if chapters.len() < c {
            chapters.resize_with(c, Vec::new);
        }
        let verses = &mut chapters[c - 1];
        if verses.len() < v {
            verses.resize_with(v, String::new);
        }
        verses[v - 1] = clean(text);
    }

    /// Number of chapters this translation has for `book`.
    pub fn chapter_count(&self, book: Book) -> usize {
        self.books[book.index()].len()
    }

    /// The text of a verse, if present.
    pub fn verse(&self, book: Book, chapter: u16, verse: u16) -> Option<&str> {
        let chapters = &self.books[book.index()];
        let verses = chapters.get(usize::from(chapter).checked_sub(1)?)?;
        let text = verses.get(usize::from(verse).checked_sub(1)?)?;
        (!text.is_empty()).then_some(text.as_str())
    }

    /// The verses a reference covers, as `(verse number, text)`. Verses the
    /// translation doesn't have are skipped, and a range running past the end
    /// of the chapter stops there. Empty if nothing matched.
    pub fn passage(&self, reference: &Reference) -> Vec<(u16, &str)> {
        let Some(verses) = usize::from(reference.chapter)
            .checked_sub(1)
            .and_then(|c| self.books[reference.book.index()].get(c))
        else {
            return Vec::new();
        };
        let last = u16::try_from(verses.len()).unwrap_or(u16::MAX);
        let ranges = if reference.verses.is_empty() {
            vec![(1, last)]
        } else {
            reference.verses.clone()
        };
        ranges
            .into_iter()
            .flat_map(|(start, end)| start..=end.min(last))
            .filter_map(|v| {
                self.verse(reference.book, reference.chapter, v)
                    .map(|t| (v, t))
            })
            .collect()
    }

    /// Every verse, in canonical order: `(book, chapter, verse, text)`.
    pub fn verses(&self) -> impl Iterator<Item = (Book, u16, u16, &str)> {
        Book::all().flat_map(move |book| {
            self.books[book.index()]
                .iter()
                .enumerate()
                .flat_map(move |(c, verses)| {
                    verses
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| !t.is_empty())
                        .map(move |(v, text)| (book, (c + 1) as u16, (v + 1) as u16, text.as_str()))
                })
        })
    }

    /// Number of verses with text.
    pub fn verse_count(&self) -> usize {
        self.verses().count()
    }

    /// Parses the JSON layout described in the [crate docs](crate).
    pub fn from_json(translation: Translation, json: &str) -> Result<Self, LoadError> {
        let books: Vec<JsonBook> = serde_json::from_str(json.trim_start_matches('\u{feff}'))?;
        if books.len() != Book::COUNT {
            return Err(LoadError::BookCount(books.len()));
        }
        let mut bible = Self::new(translation);
        for (book, data) in Book::all().zip(books) {
            bible.books[book.index()] = data
                .chapters
                .into_iter()
                .map(|verses| verses.iter().map(|t| clean(t)).collect())
                .collect();
        }
        Ok(bible)
    }

    /// Parses the TSV layout described in the [crate docs](crate).
    pub fn from_tsv(translation: Translation, tsv: &str) -> Result<Self, LoadError> {
        let mut bible = Self::new(translation);
        for (i, line) in tsv.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let error = |reason: &str| LoadError::Tsv {
                line: i + 1,
                reason: reason.to_owned(),
            };
            let mut fields = line.splitn(4, '\t');
            let (Some(book), Some(chapter), Some(verse), Some(text)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                return Err(error(
                    "expected book, chapter, verse and text separated by tabs",
                ));
            };
            let book = Book::from_osis(book.trim())
                .or_else(|| Book::find(book))
                .ok_or_else(|| error("unknown book"))?;
            let chapter = chapter
                .trim()
                .parse()
                .map_err(|_| error("bad chapter number"))?;
            let verse = verse
                .trim()
                .parse()
                .map_err(|_| error("bad verse number"))?;
            bible.set_verse(book, chapter, verse, text);
        }
        Ok(bible)
    }

    /// Loads a `.json` or `.tsv` file.
    pub fn load(translation: Translation, path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let path = path.as_ref();
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        let text = || std::fs::read_to_string(path);
        match extension.as_deref() {
            Some("json") => Self::from_json(translation, &text()?),
            Some("tsv") => Self::from_tsv(translation, &text()?),
            _ => Err(LoadError::UnknownFormat(path.to_path_buf())),
        }
    }
}

/// Collapses all whitespace (including newlines) into single spaces.
fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../fixtures/sample.tsv");

    fn sample() -> Bible {
        Bible::from_tsv(
            Translation::new("RV1909", "Reina-Valera 1909", Language::Spanish),
            SAMPLE,
        )
        .unwrap()
    }

    fn reference(text: &str) -> Reference {
        Reference::parse(text).unwrap()
    }

    #[test]
    fn loads_tsv() {
        let bible = sample();
        assert_eq!(bible.verse_count(), 8);
        let john = Book::from_osis("John").unwrap();
        assert!(
            bible
                .verse(john, 3, 16)
                .unwrap()
                .starts_with("Porque de tal manera")
        );
        assert_eq!(bible.verse(john, 3, 99), None);
        assert_eq!(bible.chapter_count(john), 3);
    }

    #[test]
    fn passages_clip_to_what_exists() {
        let bible = sample();
        let verses: Vec<u16> = bible
            .passage(&reference("Juan 3:16-40"))
            .iter()
            .map(|(v, _)| *v)
            .collect();
        assert_eq!(verses, [16, 17]);
        assert_eq!(bible.passage(&reference("Sal 23")).len(), 3);
        assert!(bible.passage(&reference("Juan 9:1")).is_empty());
    }

    #[test]
    fn tsv_errors_name_the_line() {
        let t = Translation::new("X", "X", Language::English);
        let err = Bible::from_tsv(t.clone(), "# ok\nJohn\t3\t16\tok\nNope\t1\t1\tx").unwrap_err();
        assert!(matches!(err, LoadError::Tsv { line: 3, .. }), "{err}");
        assert!(Bible::from_tsv(t, "John 3 16").is_err());
    }

    #[test]
    fn loads_json_with_bom() {
        let mut books = vec![r#"{"chapters": []}"#.to_owned(); Book::COUNT];
        books[0] = r#"{"abbrev": "gn", "chapters": [["In the  beginning\n God created."]]}"#.into();
        let json = format!("\u{feff}[{}]", books.join(","));
        let bible = Bible::from_json(
            Translation::new("KJV", "King James", Language::English),
            &json,
        )
        .unwrap();
        assert_eq!(
            bible.verse(Book::from_index(0).unwrap(), 1, 1),
            Some("In the beginning God created.")
        );

        let short = Bible::from_json(Translation::new("X", "X", Language::English), "[]");
        assert!(matches!(short, Err(LoadError::BookCount(0))));
    }

    #[test]
    fn load_picks_format_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rv1909.tsv");
        std::fs::write(&path, SAMPLE).unwrap();
        let t = Translation::new("RV1909", "Reina-Valera 1909", Language::Spanish);
        assert_eq!(Bible::load(t.clone(), &path).unwrap().verse_count(), 8);
        assert!(matches!(
            Bible::load(t, dir.path().join("x.xml")),
            Err(LoadError::UnknownFormat(_))
        ));
    }
}
