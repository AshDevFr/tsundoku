//! Nyaa.si implementation of [`td_source::DiscoverySource`].

pub mod detail;
pub mod fetcher;
pub mod links;
pub mod listing;
pub mod parser;
pub mod search;
pub mod source;

pub use search::{NyaaSearch, NyaaSearchConfig};
pub use source::{NyaaSource, NyaaSourceConfig};

/// `source_kind` value emitted by every Nyaa source. Persisted to
/// `releases.source_kind` and used by the registry to pick the constructor.
pub const SOURCE_KIND: &str = "nyaa";
