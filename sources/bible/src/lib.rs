//! The Bible as a chorquest [`Source`](chorquest_resource::Source).
//!
//! A [`BibleSource`] serves one offline [`Bible`] translation in two ways:
//!
//! - **Indexed**: every verse becomes a resource (`Juan 3:16`), so the
//!   engine's index can find verses by their *text* ("de tal manera amó").
//! - **Live**: queries that look like references (`Juan 3:16-18`,
//!   `jn 3 16`, `1 Co 13:4-7`, `primera de juan 4`, `Sal 23`, several joined
//!   with `;`) are parsed and answered with the whole passage, one slide per
//!   verse.
//!
//! Book names are understood in Spanish and English, with or without
//! accents, abbreviated, or with ordinals spelled out. See [`Book::find`]
//! and [`Reference::parse`].
//!
//! # Getting a translation
//!
//! No text is bundled: most modern translations are copyrighted. Load a file
//! you are allowed to use (Reina-Valera 1909 and the King James Version are
//! public domain) in either format:
//!
//! - **JSON**: an array of the 66 books in canonical order, each an object
//!   with a `chapters` field holding arrays of verse strings. Other fields are
//!   ignored. This layout is used by several open datasets.
//!   `[{"chapters": [["In the beginning...", "..."], ...]}, ...]`
//! - **TSV**: one verse per line: `book<TAB>chapter<TAB>verse<TAB>text`,
//!   where `book` is an OSIS code (`John`, `1Cor`) or any name [`Book::find`]
//!   understands. Empty lines and lines starting with `#` are skipped. This is
//!   the easiest target when converting from other formats.

mod bible;
mod books;
mod reference;
mod source;

pub use bible::{Bible, LoadError, Translation};
pub use books::{Book, Language};
pub use reference::Reference;
pub use source::BibleSource;
