//! Metadata-provider abstraction.
//!
//! A [`MetadataProvider`] knows how to fetch canonical series metadata from
//! one source (MangaBaka, AniList, MAL, ...). Implementations live in
//! separate crates (`td-metadata-mangabaka`, etc.); this crate defines only
//! the trait, the canonical response shape, the in-process registry, and
//! the error type.
//!
//! The pattern is a simplified port of Codex's plugin system: same canonical
//! response shape and the same `(series_id, provider, external_id)` identity
//! model, but in-process Rust traits rather than subprocess JSON-RPC. We are
//! a single binary by a single author; extensibility means "write a new crate",
//! not "ship a multi-language SDK".

pub mod error;
pub mod provider;
pub mod registry;
pub mod types;

pub use error::{MetadataError, MetadataResult};
pub use provider::MetadataProvider;
pub use registry::{MetadataRegistry, MetadataRegistryBuilder, RegistryError};
pub use types::{
    ForeignId, RefreshStatus, RefreshSummary, SearchHit, SeriesKind, SeriesMetadata, SeriesStatus,
};
