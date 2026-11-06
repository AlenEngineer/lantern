/// An identifier did not follow the expected format.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid id {0:?}")]
pub struct IdError(pub String);

/// A [`Source`](crate::Source) failed to scan or search.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SourceError {
    /// Reading from disk failed.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// The source cannot be reached right now (offline, folder missing,
    /// API key not configured...).
    #[error("source unavailable: {0}")]
    Unavailable(String),
    /// Anything else, for sources with their own error types.
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}
