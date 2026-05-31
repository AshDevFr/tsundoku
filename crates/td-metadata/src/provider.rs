//! The [`MetadataProvider`] trait.
//!
//! Implementations live in separate crates (e.g. `td-metadata-mangabaka`).
//! The trait surface is small on purpose: anything provider-specific (offline
//! dump format, cache schema, API auth shape) is an implementation detail
//! behind these methods.

use async_trait::async_trait;

use crate::error::MetadataResult;
use crate::types::{RefreshSummary, SearchHit, SeriesMetadata};

#[async_trait]
pub trait MetadataProvider: Send + Sync {
    /// Stable identifier used as the namespace in
    /// `series_external_ids.provider` and as the `[providers.<id>]` key in
    /// config. Lowercase, no spaces.
    fn id(&self) -> &str;

    fn display_name(&self) -> &str;

    /// Fetch full metadata by this provider's external ID. Implementations
    /// may consult a local cache (offline dump, in-memory LRU) before the
    /// network. `Ok(None)` means "no such series", which is distinct from
    /// "the provider failed to answer".
    async fn get(&self, external_id: &str) -> MetadataResult<Option<SeriesMetadata>>;

    /// Free-text search. Used by the review UI and by the resolver's
    /// fuzzy-title fallback. Implementations decide whether to consult
    /// offline data, the network, or both.
    async fn search(&self, query: &str, limit: u32) -> MetadataResult<Vec<SearchHit>>;

    /// Resolve by another provider's ID. The resolver calls this on the
    /// active provider when a release surfaces an external link to a
    /// different provider (e.g. a Nyaa post links to MangaUpdates and
    /// MangaBaka resolves it via `/v1/source/manga-updates/{id}`).
    ///
    /// Default impl returns `Ok(None)`: providers without foreign-ID
    /// cross-resolution simply do not participate in this step.
    async fn resolve_by_foreign_id(
        &self,
        _foreign_provider: &str,
        _foreign_id: &str,
    ) -> MetadataResult<Option<SeriesMetadata>> {
        Ok(None)
    }

    /// Canonical provider ids this provider can cross-resolve through
    /// [`Self::resolve_by_foreign_id`]. Reported to the review UI so the
    /// "Search provider" modal can offer a foreign-ID lookup against the
    /// active provider (e.g. paste a MangaUpdates id, resolve it via
    /// MangaBaka). Default empty: providers without cross-resolution offer
    /// only native id lookup.
    fn foreign_sources(&self) -> &'static [&'static str] {
        &[]
    }

    /// Refresh the provider's local cache (e.g. download an offline dump).
    /// Default impl returns [`crate::RefreshStatus::NotSupported`] so the
    /// generic `tsundoku refresh-provider-cache` CLI can iterate the
    /// registry without per-provider conditionals.
    async fn refresh_cache(&self) -> MetadataResult<RefreshSummary> {
        Ok(RefreshSummary::not_supported(self.id()))
    }

    /// Whether an offline cache (dump, on-disk index, etc.) is currently
    /// loaded and ready to serve reads. Reported by the admin UI so the
    /// operator can tell at a glance whether a provider is hitting disk or
    /// going straight to the network. Providers with no offline path return
    /// the default `false`.
    async fn offline_cache_loaded(&self) -> bool {
        false
    }
}
