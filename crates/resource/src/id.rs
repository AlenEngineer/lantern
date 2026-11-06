use std::fmt;

use serde::{Deserialize, Serialize};

use crate::IdError;

/// Identifies a [`Source`](crate::Source), e.g. `local`, `youtube`, `bible-rvr1960`.
///
/// Only lowercase ASCII letters, digits, `-` and `_` are allowed. In
/// particular it never contains `:`, so it can safely prefix a [`ResourceId`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SourceId(String);

impl SourceId {
    /// Validates and wraps a source id.
    pub fn new(id: impl Into<String>) -> Result<Self, IdError> {
        let id = id.into();
        let valid = !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
        if valid {
            Ok(Self(id))
        } else {
            Err(IdError(id))
        }
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SourceId {
    type Error = IdError;
    fn try_from(value: String) -> Result<Self, IdError> {
        Self::new(value)
    }
}

impl From<SourceId> for String {
    fn from(id: SourceId) -> String {
        id.0
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Globally unique id of a [`Resource`](crate::Resource): `"<source>:<key>"`.
///
/// The key is whatever the source uses to tell its items apart (a file path,
/// a video id, a verse reference...). Ids are stable: the same item scanned
/// twice must get the same id, which is what lets the index replace it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ResourceId(String);

impl ResourceId {
    /// Builds the id of `key` within `source`.
    pub fn new(source: &SourceId, key: &str) -> Self {
        Self(format!("{source}:{key}"))
    }

    /// The source part of the id.
    pub fn source(&self) -> &str {
        self.split().0
    }

    /// The source-specific part of the id.
    pub fn key(&self) -> &str {
        self.split().1
    }

    /// Whether this resource was produced by `source`.
    pub fn belongs_to(&self, source: &SourceId) -> bool {
        self.source() == source.as_str()
    }

    /// The full id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn split(&self) -> (&str, &str) {
        // Guaranteed by construction (`new` and `try_from`).
        self.0.split_once(':').unwrap_or((&self.0, ""))
    }
}

impl TryFrom<String> for ResourceId {
    type Error = IdError;
    fn try_from(value: String) -> Result<Self, IdError> {
        match value.split_once(':') {
            Some((source, _)) if SourceId::new(source).is_ok() => Ok(Self(value)),
            _ => Err(IdError(value)),
        }
    }
}

impl From<ResourceId> for String {
    fn from(id: ResourceId) -> String {
        id.0
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_ids_are_validated() {
        assert!(SourceId::new("local").is_ok());
        assert!(SourceId::new("bible-rvr_1960").is_ok());
        assert!(SourceId::new("").is_err());
        assert!(SourceId::new("Local").is_err());
        assert!(SourceId::new("a:b").is_err());
    }

    #[test]
    fn resource_id_splits_on_first_colon() {
        let source = SourceId::new("link").unwrap();
        let id = ResourceId::new(&source, "https://example.com/a:b");
        assert_eq!(id.source(), "link");
        assert_eq!(id.key(), "https://example.com/a:b");
        assert!(id.belongs_to(&source));
    }

    #[test]
    fn resource_id_rejects_malformed_strings() {
        assert!(ResourceId::try_from("no-colon".to_owned()).is_err());
        assert!(ResourceId::try_from("Bad Source:x".to_owned()).is_err());
        assert!(ResourceId::try_from("local:/x".to_owned()).is_ok());
    }
}
