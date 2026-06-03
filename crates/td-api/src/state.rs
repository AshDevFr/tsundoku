//! Application state shared by every handler.
//!
//! Built once in the binary's `serve` command and passed into [`crate::router`].
//! Cheap to clone (everything is `Arc`).

use std::path::PathBuf;
use std::sync::Arc;

use sea_orm::DatabaseConnection;
use serde::Serialize;
use td_config::{
    AuthConfig, CodexConfig, DownloadConfig, IngestionConfig, MetadataConfig, ProvidersConfig,
    SourceConfig,
};
use td_download::DownloadClient;
use td_metadata::MetadataRegistry;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;
use td_scheduler::JobLocks;
use td_source::SourceRegistry;
use tokio::sync::broadcast;
use utoipa::ToSchema;

// Job-lifecycle event types live in `td-scheduler` so cron-driven jobs
// can construct and emit them without depending on this crate. Re-exported
// here for handler / test convenience and registered in `docs::ApiDoc`.
pub use td_scheduler::{JOB_EVENT_BUFFER, JobEvent, JobKind, JobPhase, JobProgress, JobResult};

/// Currently-running marker hung off each source / provider listing entry
/// so the admin UI can render the "RUNNING…" pill straight from the DTO
/// on a fresh page load, without waiting for the SSE channel to replay
/// state it never had.
///
/// Hydrated from the matching `*_runs` row whose `status = 'running'`.
/// `progress` is populated when the in-flight row's `progress_current` /
/// `progress_total` / `progress_phase` columns are non-`NULL` (i.e. the
/// job is reporting progress); jobs that don't report leave it `None`
/// and the pill shows the binary "is in flight" state.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InFlight {
    /// Epoch seconds the in-flight run started at.
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<JobProgress>,
}

impl InFlight {
    /// Build an `InFlight` from a repo `InFlightRunRow`. `progress` is set
    /// when the row carries at least a `progress_current` checkpoint;
    /// missing-total / missing-phase are passed through as `None` so the
    /// UI can render a `current`-only fraction-free pill.
    pub fn from_row(row: td_db::repos::run_metrics_repo::InFlightRunRow) -> Self {
        let progress = row.progress_current.map(|current| JobProgress {
            current: current as u64,
            total: row.progress_total.map(|t| t as u64),
            phase: row.progress_phase,
        });
        Self {
            started_at: row.started_at,
            progress,
        }
    }
}

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
    /// Snapshot of the `[metadata]` block. Today used only by the
    /// bulk series-refresh endpoint to pick up `batch_size` and
    /// `min_age_days`; the active-provider id lives on the registry.
    pub metadata_config: Arc<MetadataConfig>,
    /// Title cleaner shared with the scheduler. Built once at startup
    /// from the built-in keyword list plus
    /// `ingestion.cleanup.extra_format_keywords`. Tests fall back to a
    /// defaults-only cleaner.
    pub query_builder: Arc<QueryBuilder>,
    /// Shared with the scheduler. `None` in tests that don't need legacy
    /// MangaUpdates URL translation; resolver runs on the API retry path
    /// honor this when building their `Resolver`.
    pub mangaupdates_redirector: Option<Arc<MangaUpdatesRedirector>>,
    /// Broadcast channel for manual-trigger lifecycle events. The SSE
    /// endpoint subscribes a fresh receiver per connection; manual
    /// trigger handlers publish via [`AppState::send_job_event`]. Cron
    /// jobs intentionally do **not** publish here, this channel is
    /// the "user is staring at the screen" signal, not full audit log.
    pub job_events: broadcast::Sender<JobEvent>,
    /// Root for the cover-image proxy cache. `None` disables the
    /// `/api/v1/covers/*` endpoints (they respond 503). The `serve`
    /// command populates this from `cfg.storage.paths().cover_cache_dir`;
    /// test scaffolds default to `None`.
    pub cover_cache_dir: Option<PathBuf>,
    /// Snapshot of the `[codex]` config. Drives the status endpoint's
    /// `enabled` flag and (in a later phase) the series deep-link base URL.
    pub codex: Arc<CodexConfig>,
    /// Codex client for the manual `POST /codex/refresh` trigger. `None` when
    /// the integration is disabled; the endpoint then responds 503. Shared
    /// with the scheduler so the manual trigger and the cron drive the same
    /// client under the same lock.
    pub codex_client: Option<Arc<td_codex::CodexClient>>,
    /// Snapshot of the `[download]` config. Drives the `GET /download/status`
    /// endpoint's `enabled`/`kind` and the send handler's per-send defaults.
    pub download: Arc<DownloadConfig>,
    /// Torrent client for the `POST /releases/{id}/send-to-client` action.
    /// `None` when the integration is disabled; the endpoint then responds 503
    /// `Misconfigured`.
    pub download_client: Option<Arc<dyn DownloadClient>>,
}

impl AppState {
    /// Best-effort publish. We don't propagate the `SendError` because
    /// the only failure mode is "no receivers connected," which is the
    /// normal state when nobody has the admin page open.
    pub fn send_job_event(&self, event: JobEvent) {
        let _ = self.job_events.send(event);
    }
}
