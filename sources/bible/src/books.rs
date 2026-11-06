use std::fmt;
use std::sync::OnceLock;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Languages book names can be displayed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    /// English.
    English,
    /// Spanish.
    Spanish,
}

/// (OSIS code, English name, Spanish name, extra abbreviations in either language)
type Entry = (
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
);

/// The 66 books of the Protestant canon, in order.
const BOOKS: [Entry; 66] = [
    ("Gen", "Genesis", "Génesis", &["gn"]),
    ("Exod", "Exodus", "Éxodo", &["ex"]),
    ("Lev", "Leviticus", "Levítico", &["lv"]),
    ("Num", "Numbers", "Números", &["nm", "nu"]),
    ("Deut", "Deuteronomy", "Deuteronomio", &["dt"]),
    ("Josh", "Joshua", "Josué", &["jos"]),
    ("Judg", "Judges", "Jueces", &["jue", "jdg", "jc"]),
    ("Ruth", "Ruth", "Rut", &["rt"]),
    ("1Sam", "1 Samuel", "1 Samuel", &["1s"]),
    ("2Sam", "2 Samuel", "2 Samuel", &["2s"]),
    ("1Kgs", "1 Kings", "1 Reyes", &["1r", "1re", "1ki"]),
    ("2Kgs", "2 Kings", "2 Reyes", &["2r", "2re", "2ki"]),
    ("1Chr", "1 Chronicles", "1 Crónicas", &["1cr"]),
    ("2Chr", "2 Chronicles", "2 Crónicas", &["2cr"]),
    ("Ezra", "Ezra", "Esdras", &["esd", "ezr"]),
    ("Neh", "Nehemiah", "Nehemías", &["ne"]),
    ("Esth", "Esther", "Ester", &["est"]),
    ("Job", "Job", "Job", &["jb"]),
    (
        "Ps",
        "Psalms",
        "Salmos",
        &["sal", "sl", "salmo", "psalm", "psa"],
    ),
    ("Prov", "Proverbs", "Proverbios", &["pr", "pv"]),
    ("Eccl", "Ecclesiastes", "Eclesiastés", &["ec", "ecl", "qoh"]),
    (
        "Song",
        "Song of Songs",
        "Cantares",
        &[
            "cnt",
            "cant",
            "cantar de los cantares",
            "song of solomon",
            "sos",
        ],
    ),
    ("Isa", "Isaiah", "Isaías", &["is"]),
    ("Jer", "Jeremiah", "Jeremías", &["jr"]),
    ("Lam", "Lamentations", "Lamentaciones", &["lm"]),
    ("Ezek", "Ezekiel", "Ezequiel", &["ez"]),
    ("Dan", "Daniel", "Daniel", &["dn"]),
    ("Hos", "Hosea", "Oseas", &["os"]),
    ("Joel", "Joel", "Joel", &["jl"]),
    ("Amos", "Amos", "Amós", &["am"]),
    ("Obad", "Obadiah", "Abdías", &["abd", "ob"]),
    ("Jonah", "Jonah", "Jonás", &["jon"]),
    ("Mic", "Micah", "Miqueas", &["mi", "mq"]),
    ("Nah", "Nahum", "Nahúm", &["na"]),
    ("Hab", "Habakkuk", "Habacuc", &["hab"]),
    ("Zeph", "Zephaniah", "Sofonías", &["sof"]),
    ("Hag", "Haggai", "Hageo", &["hag"]),
    ("Zech", "Zechariah", "Zacarías", &["zac"]),
    ("Mal", "Malachi", "Malaquías", &["mal"]),
    ("Matt", "Matthew", "Mateo", &["mt"]),
    ("Mark", "Mark", "Marcos", &["mr", "mc", "mk"]),
    ("Luke", "Luke", "Lucas", &["lc", "lk"]),
    ("John", "John", "Juan", &["jn"]),
    (
        "Acts",
        "Acts",
        "Hechos",
        &["hch", "hech", "hechos de los apostoles"],
    ),
    ("Rom", "Romans", "Romanos", &["ro", "rm"]),
    ("1Cor", "1 Corinthians", "1 Corintios", &["1co"]),
    ("2Cor", "2 Corinthians", "2 Corintios", &["2co"]),
    ("Gal", "Galatians", "Gálatas", &["ga"]),
    ("Eph", "Ephesians", "Efesios", &["ef"]),
    ("Phil", "Philippians", "Filipenses", &["fil", "flp", "php"]),
    ("Col", "Colossians", "Colosenses", &["col"]),
    (
        "1Thess",
        "1 Thessalonians",
        "1 Tesalonicenses",
        &["1ts", "1tes", "1th"],
    ),
    (
        "2Thess",
        "2 Thessalonians",
        "2 Tesalonicenses",
        &["2ts", "2tes", "2th"],
    ),
    ("1Tim", "1 Timothy", "1 Timoteo", &["1ti"]),
    ("2Tim", "2 Timothy", "2 Timoteo", &["2ti"]),
    ("Titus", "Titus", "Tito", &["tit"]),
    ("Phlm", "Philemon", "Filemón", &["flm", "phm"]),
    ("Heb", "Hebrews", "Hebreos", &["he"]),
    ("Jas", "James", "Santiago", &["stg", "stgo", "sant", "jm"]),
    ("1Pet", "1 Peter", "1 Pedro", &["1p", "1pe"]),
    ("2Pet", "2 Peter", "2 Pedro", &["2p", "2pe"]),
    ("1John", "1 John", "1 Juan", &["1jn"]),
    ("2John", "2 John", "2 Juan", &["2jn"]),
    ("3John", "3 John", "3 Juan", &["3jn"]),
    ("Jude", "Jude", "Judas", &["jud"]),
    (
        "Rev",
        "Revelation",
        "Apocalipsis",
        &["ap", "apoc", "rv", "revelations", "apocalypse"],
    ),
];

/// A book of the Bible, in canonical order (Genesis is 0, Revelation is 65).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Book(u8);

impl Book {
    /// How many books there are.
    pub const COUNT: usize = BOOKS.len();

    /// Every book, in order.
    pub fn all() -> impl Iterator<Item = Book> {
        (0..Self::COUNT).map(|i| Book(i as u8))
    }

    /// The book at a canonical position (0-based).
    pub fn from_index(index: usize) -> Option<Book> {
        (index < Self::COUNT).then_some(Book(index as u8))
    }

    /// Canonical position (0-based).
    pub fn index(self) -> usize {
        usize::from(self.0)
    }

    /// The OSIS code, e.g. `"1Cor"`. Used in resource ids.
    pub fn osis(self) -> &'static str {
        BOOKS[self.index()].0
    }

    /// The display name in `language`.
    pub fn name(self, language: Language) -> &'static str {
        let (_, en, es, _) = BOOKS[self.index()];
        match language {
            Language::English => en,
            Language::Spanish => es,
        }
    }

    /// Looks up an OSIS code, ignoring case.
    pub fn from_osis(code: &str) -> Option<Book> {
        Self::all().find(|b| b.osis().eq_ignore_ascii_case(code))
    }

    /// Understands a book name the way people type it: in Spanish or English,
    /// with or without accents and dots, abbreviated (`jn`, `1 co`, `Sal`),
    /// with ordinals (`primera de Juan`, `II Reyes`, `1st John`) or the
    /// `San`/`St.` prefix (`San Juan`). An unambiguous prefix works as well
    /// (`gene`, `filip`). Ambiguous input (`ju`: Juan, Jueces, Judas...)
    /// returns `None`.
    pub fn find(name: &str) -> Option<Book> {
        let key = book_key(name);
        if key.is_empty() {
            return None;
        }
        let names = names();
        let start = names.partition_point(|(n, _)| n.as_str() < key.as_str());
        let mut matching = names[start..]
            .iter()
            .take_while(|(n, _)| n.starts_with(&key));

        let (first_name, first_book) = matching.next()?;
        if *first_name == key {
            return Some(*first_book);
        }
        if key.chars().count() < 2 {
            return None;
        }
        matching
            .all(|(_, b)| b == first_book)
            .then_some(*first_book)
    }
}

impl fmt::Display for Book {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.osis())
    }
}

/// Every known spelling of every book, normalized and sorted.
fn names() -> &'static [(String, Book)] {
    static NAMES: OnceLock<Vec<(String, Book)>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names: Vec<(String, Book)> = Book::all()
            .flat_map(|book| {
                let (osis, en, es, aliases) = BOOKS[book.index()];
                [osis, en, es]
                    .into_iter()
                    .chain(aliases.iter().copied())
                    .map(move |name| (normalize(name), book))
            })
            .collect();
        names.sort();
        names.dedup();
        names
    })
}

/// Lowercase, no accents, no punctuation or spaces: `"1 Crónicas"` → `"1cronicas"`.
fn normalize(name: &str) -> String {
    fold(name).chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Like [`normalize`], but first rewrites ordinals and drops `San`/`St.`.
fn book_key(input: &str) -> String {
    let folded = fold(input).replace('.', " ");
    let mut words: Vec<&str> = folded.split_whitespace().collect();
    if words.len() > 1 {
        if let Some(number) = ordinal(words[0]) {
            words[0] = number;
            if matches!(words[1], "de" | "of") && words.len() > 2 {
                words.remove(1);
            }
        } else if matches!(words[0], "san" | "st" | "saint") {
            words.remove(0);
        }
    }
    normalize(&words.concat())
}

fn ordinal(word: &str) -> Option<&'static str> {
    Some(match word {
        "1" | "i" | "1a" | "1o" | "1ra" | "1ro" | "1er" | "1era" | "1st" | "primera"
        | "primero" | "primer" | "first" => "1",
        "2" | "ii" | "2a" | "2o" | "2da" | "2do" | "2nd" | "segunda" | "segundo" | "second" => "2",
        "3" | "iii" | "3a" | "3o" | "3ra" | "3ro" | "3er" | "3era" | "3rd" | "tercera"
        | "tercero" | "tercer" | "third" => "3",
        _ => return None,
    })
}

pub(crate) fn fold(text: &str) -> String {
    text.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(name: &str) -> Option<&'static str> {
        Book::find(name).map(Book::osis)
    }

    #[test]
    fn canonical_order() {
        assert_eq!(Book::COUNT, 66);
        assert_eq!(Book::from_index(0).unwrap().osis(), "Gen");
        assert_eq!(
            Book::from_index(42).unwrap().name(Language::Spanish),
            "Juan"
        );
        assert_eq!(
            Book::from_index(65).unwrap().name(Language::English),
            "Revelation"
        );
        assert!(Book::from_index(66).is_none());
    }

    #[test]
    fn full_names_in_both_languages() {
        assert_eq!(find("Juan"), Some("John"));
        assert_eq!(find("john"), Some("John"));
        assert_eq!(find("Génesis"), Some("Gen"));
        assert_eq!(find("genesis"), Some("Gen"));
        assert_eq!(find("Apocalipsis"), Some("Rev"));
        assert_eq!(find("Cantar de los Cantares"), Some("Song"));
    }

    #[test]
    fn numbered_books_and_ordinals() {
        for name in [
            "1 Juan",
            "1juan",
            "1 Jn",
            "I Juan",
            "primera de Juan",
            "1ra Juan",
            "1st John",
        ] {
            assert_eq!(find(name), Some("1John"), "{name}");
        }
        assert_eq!(find("Juan"), Some("John"));
        assert_eq!(find("2 Co"), Some("2Cor"));
        assert_eq!(find("segunda corintios"), Some("2Cor"));
        assert_eq!(find("III Juan"), Some("3John"));
    }

    #[test]
    fn abbreviations_and_prefixes() {
        assert_eq!(find("Sal"), Some("Ps"));
        assert_eq!(find("Salmo"), Some("Ps"));
        assert_eq!(find("Mt."), Some("Matt"));
        assert_eq!(find("Hch"), Some("Acts"));
        assert_eq!(find("Ro"), Some("Rom"));
        assert_eq!(find("filip"), Some("Phil"));
        assert_eq!(find("Stgo"), Some("Jas"));
        assert_eq!(find("Santiago"), Some("Jas"));
    }

    #[test]
    fn san_prefix() {
        assert_eq!(find("San Juan"), Some("John"));
        assert_eq!(find("San Mateo"), Some("Matt"));
        assert_eq!(find("St. John"), Some("John"));
    }

    #[test]
    fn ambiguous_or_unknown() {
        assert_eq!(find("ju"), None);
        assert_eq!(find("j"), None);
        assert_eq!(find(""), None);
        assert_eq!(find("tobias"), None);
    }

    #[test]
    fn no_spelling_points_to_two_books() {
        let names = names();
        for pair in names.windows(2) {
            assert!(pair[0].0 != pair[1].0, "{:?} is ambiguous", pair[0].0);
        }
    }
}
