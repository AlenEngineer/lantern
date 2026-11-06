//! The shared vocabulary of chorquest.
//!
//! Everything the engine handles is a [`Resource`]: something that can be put
//! on stage, no matter where it came from. Resources are produced by
//! [`Source`]s, and implementing [`Source`] is the only thing needed to teach
//! chorquest about a new place to find media (a folder, YouTube, a Bible
//! API, a hymnal...).
//!
//! This crate is deliberately small and dependency-light: every other crate
//! and every extension depends on it.

mod error;
mod id;
mod kind;
mod location;
mod query;
mod record;
mod source;

pub use error::{IdError, SourceError};
pub use id::{ResourceId, SourceId};
pub use kind::Kind;
pub use location::Location;
pub use query::{DEFAULT_LIMIT, Query};
pub use record::{Hit, Resource};
pub use source::{Capabilities, Source};
