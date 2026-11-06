use serde::{Deserialize, Serialize};

use crate::{Kind, Resource, SourceId};

/// How many results a query returns unless told otherwise.
pub const DEFAULT_LIMIT: usize = 50;

/// A search request, as typed by the operator plus optional filters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Query {
    /// Free text. May be empty, which means "browse everything that passes the filters".
    pub text: String,
    /// Only return these kinds. Empty means any kind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<Kind>,
    /// Only ask these sources. Empty means every source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceId>,
    /// Maximum number of results per origin (the index, or each live source).
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    DEFAULT_LIMIT
}

impl Query {
    /// A query for `text` with no filters.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kinds: Vec::new(),
            sources: Vec::new(),
            limit: DEFAULT_LIMIT,
        }
    }

    /// Adds a kind to the kind filter.
    pub fn with_kind(mut self, kind: Kind) -> Self {
        self.kinds.push(kind);
        self
    }

    /// Adds a source to the source filter.
    pub fn with_source(mut self, source: SourceId) -> Self {
        self.sources.push(source);
        self
    }

    /// Sets the result limit.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Whether `source` should be asked at all.
    pub fn wants_source(&self, source: &SourceId) -> bool {
        self.sources.is_empty() || self.sources.contains(source)
    }

    /// Whether `resource` passes the kind and source filters (the text is not checked).
    pub fn accepts(&self, resource: &Resource) -> bool {
        let kind_ok = self.kinds.is_empty() || self.kinds.contains(&resource.kind);
        let source_ok =
            self.sources.is_empty() || self.sources.iter().any(|s| resource.id.belongs_to(s));
        kind_ok && source_ok
    }
}
