//! Canonical response types.
//!
//! Every provider implementation maps its native payload into these shapes.
//! The resolver, persistence layer, and API never see provider-specific
//! types: they only see [`SeriesMetadata`], [`SearchHit`], etc.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Provider-agnostic series metadata. Returned by [`crate::MetadataProvider::get`]
/// and persisted into the `series` row + `series_external_ids` mapping table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesMetadata {
    /// The provider's own external ID for this series. Combined with the
    /// provider's `id()` to form a `series_external_ids` entry.
    pub external_id: String,
    pub canonical_title: String,
    #[serde(default)]
    pub alternate_titles: Vec<String>,
    pub kind: Option<SeriesKind>,
    pub status: Option<SeriesStatus>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub external_url: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Cross-references this provider knows about. The resolver persists
    /// these into `series_external_ids` so future releases pointing at any
    /// of these foreign IDs short-circuit to the same series row.
    #[serde(default)]
    pub foreign_ids: Vec<ForeignId>,
    /// Full provider payload, persisted on `series.metadata_json` for
    /// audit and for re-derivation when the canonical type evolves.
    pub raw: serde_json::Value,
    /// Hash of `raw`. Upsert callers skip writes when the hash is unchanged.
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesKind {
    Manga,
    Manhwa,
    Manhua,
    Novel,
    OneShot,
    /// Original English-language; some providers expose this distinctly.
    Oel,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesStatus {
    Ongoing,
    Completed,
    Hiatus,
    Cancelled,
    Upcoming,
    Unknown,
}

/// A pointer at another provider's external ID for the same series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignId {
    /// Canonical provider id (e.g. `"mangaupdates"`, `"anilist"`, `"mal"`,
    /// `"mangadex"`). Maps to `series_external_ids.provider`.
    pub provider: String,
    /// The foreign provider's ID, stringified. Maps to
    /// `series_external_ids.external_id`.
    pub id: String,
    pub url: Option<String>,
}

/// One row of a search result. Returned by [`crate::MetadataProvider::search`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub external_id: String,
    pub title: String,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    /// Relevance, normalized to `[0.0, 1.0]` if the provider supplies one.
    pub score: Option<f32>,
}

/// What happened during a `refresh_cache()` call. Surfaced by the CLI and
/// the `POST /api/v1/providers/{id}/refresh-cache` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshSummary {
    pub provider: String,
    pub status: RefreshStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub bytes_downloaded: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RefreshStatus {
    /// Cache was rebuilt. `records` is the number of series ingested.
    Refreshed {
        records: u64,
        version: Option<String>,
    },
    /// Cache exists and is current. Provider declined to re-download.
    UpToDate,
    /// Provider does not maintain an offline cache. Default for the no-op
    /// `refresh_cache()` impl.
    NotSupported,
    /// Provider has a cache but skipped this run (e.g. user disabled it).
    /// The message is operator-facing.
    Skipped { message: String },
}

impl RefreshSummary {
    /// Default value used by the trait's blanket `refresh_cache()` impl for
    /// providers without an offline cache.
    pub fn not_supported(provider: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            provider: provider.into(),
            status: RefreshStatus::NotSupported,
            started_at: now,
            finished_at: now,
            bytes_downloaded: None,
        }
    }
}
