//! Application state shared by every handler.
//!
//! Built once in the binary's `serve` command and passed into [`crate::router`].
//! Cheap to clone (everything is `Arc`).

use std::sync::Arc;

use sea_orm::DatabaseConnection;
use td_config::{AuthConfig, IngestionConfig, ProvidersConfig, SourceConfig};
use td_metadata::MetadataRegistry;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_scheduler::JobLocks;
use td_source::SourceRegistry;

/// Concrete shared state passed to every handler via the axum `State`
/// extractor.
#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub sources: Arc<SourceRegistry>,
    pub metadata: Arc<MetadataRegistry>,
    pub ingestion: IngestionConfig,
    pub auth: Arc<AuthConfig>,
    /// Shared with the scheduler. Manual-trigger endpoints acquire the same
    /// per-source / per-provider mutexes the cron jobs use, so a manual
    /// kick can't race a scheduled tick.
    pub locks: Arc<JobLocks>,
    /// Snapshot of the `[[sources]]` config blocks the registry was built
    /// from. Lets the admin handlers surface cron, feed_url, etc. without
    /// a config-state table or a round trip back to the file.
    pub sources_config: Arc<Vec<SourceConfig>>,
    /// Snapshot of the `[providers]` config block, with the same role for
    /// the provider admin endpoints.
    pub providers_config: Arc<ProvidersConfig>,
    /// Shared with the scheduler. `None` in tests that don't need legacy
    /// MangaUpdates URL translation; resolver runs on the API retry path
    /// honor this when building their `Resolver`.
    pub mangaupdates_redirector: Option<Arc<MangaUpdatesRedirector>>,
}
