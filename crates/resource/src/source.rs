use crate::{Query, Resource, SourceError, SourceId};

/// What a [`Source`] can do. A source may do both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// The source can list its whole catalog. The engine calls
    /// [`Source::scan`] when (re)indexing and keeps the result in memory, so
    /// searching it costs no I/O. Use this for anything finite: local folders,
    /// a downloaded hymnal, an offline Bible.
    pub scan: bool,
    /// The source answers queries itself at search time, through
    /// [`Source::search`]. Use this for anything too big or too dynamic to
    /// index: YouTube, web search, online Bible APIs, pasted links.
    pub search: bool,
}

impl Capabilities {
    /// Scan only.
    pub const INDEXED: Self = Self {
        scan: true,
        search: false,
    };
    /// Search only.
    pub const LIVE: Self = Self {
        scan: false,
        search: true,
    };
}

/// A place resources come from. This is chorquest's extension point.
///
/// Implementations must be cheap to share between threads: the engine calls
/// them from worker threads, possibly concurrently. Methods are blocking;
/// the engine takes care of not blocking the UI.
///
/// Every resource a source returns must have an id built with
/// [`ResourceId::new`](crate::ResourceId::new)`(self.id(), ..)`, so the engine
/// knows which source owns it.
pub trait Source: Send + Sync {
    /// Stable id, used to prefix resource ids and to filter queries.
    fn id(&self) -> &SourceId;

    /// Human-readable name, for the dashboard.
    fn name(&self) -> &str;

    /// Which of [`scan`](Self::scan) and [`search`](Self::search) the engine should use.
    fn capabilities(&self) -> Capabilities;

    /// Lists every resource this source has. Called only if
    /// [`Capabilities::scan`] is set.
    fn scan(&self) -> Result<Vec<Resource>, SourceError> {
        Ok(Vec::new())
    }

    /// Answers a query, best results first. Called only if
    /// [`Capabilities::search`] is set. Returning an empty list is the normal
    /// way to say "this query is not for me".
    fn search(&self, query: &Query) -> Result<Vec<Resource>, SourceError> {
        let _ = query;
        Ok(Vec::new())
    }
}
