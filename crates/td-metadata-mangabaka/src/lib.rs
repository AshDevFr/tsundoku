//! MangaBaka implementation of the `MetadataProvider` trait.
//!
//! v1 ships an API-only path: `get`, `search`, and `resolve_by_foreign_id`
//! all hit the live MangaBaka API. `refresh_cache` returns
//! `RefreshStatus::NotSupported` until the offline dump format is confirmed
//! and the nested migrator / ingest pipeline land.
//!
//! Constants and naming follow the Codex `metadata-mangabaka` plugin
//! (`https://api.mangabaka.dev`, `x-api-key` header, `/v1/series/...`).

pub mod client;
pub mod mapping;
pub mod migration;
pub mod negative_cache;
pub mod offline;
pub mod provider;

pub use client::MangabakaClient;
pub use offline::OfflineStore;
pub use provider::MangabakaProvider;

/// Canonical provider id. Matches `[providers.mangabaka]` in config and the
/// `provider` column in `series_external_ids`.
pub const PROVIDER_ID: &str = "mangabaka";

/// Display name surfaced in the UI / CLI summaries.
pub const PROVIDER_DISPLAY_NAME: &str = "MangaBaka";
