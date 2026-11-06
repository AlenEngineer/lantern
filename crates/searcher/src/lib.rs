//! Finding things to show.
//!
//! Two layers:
//!
//! - [`Index`]: an in-memory inverted index over everything that
//!   [indexed sources](lantern_resource::Capabilities::scan) have. It folds
//!   accents and case (`cancion` finds *Canción*), matches word prefixes
//!   (`ale` finds *Aleluya*) and forgives one typo.
//! - [`Searcher`]: the registry of sources. It keeps the index up to date and
//!   runs *federated* searches: the index answers instantly, and each
//!   [live source](lantern_resource::Capabilities::search) answers on its
//!   own thread as soon as it can, through a [`SearchStream`].

mod index;
mod normalize;
mod searcher;

pub use index::Index;
pub use normalize::{fold, tokens};
pub use searcher::{
    Origin, Polled, ReindexReport, SearchError, SearchEvent, SearchStream, Searcher,
};
