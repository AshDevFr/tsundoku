//! Application state shared by every handler.
//!
//! Built once in the binary's `serve` command and passed into [`crate::router`].
//! Cheap to clone (everything is `Arc`).

use std::sync::Arc;

use sea_orm::DatabaseConnection;
use serde::Serialize;
use td_config::{AuthConfig, IngestionConfig, MetadataConfig, ProvidersConfig, SourceConfig};
use td_metadata::MetadataRegistry;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;
use td_scheduler::JobLocks;
use td_source::SourceRegistry;
use tokio::sync::broadcast;
use utoipa::ToSchema;

/// Bounded buffer for the manual-trigger event channel. Generous enough
/// that a slow client never causes the producer to lag, small enough
/// that an idle process doesn't hold meaningful memory. Per-client
/// receivers can still drop oldest events under back-pressure; that's
/// fine for ephemeral progress.
pub const JOB_EVENT_BUFFER: usize = 256;

/// Kind of work an event refers to. Stays a plain string in the
/// serialized form so the frontend can pattern-match without importing
/// the enum.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    Source,
    Provider,
    /// Bulk series-metadata refresh against the active provider.
    SeriesRefresh,
}

/// Lifecycle phase. `Started` fires after the per-key mutex was
/// acquired (so a `skipped` job emits only `Finished`).
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum JobPhase {
    Started,
    Finished,
}

/// Compact result payload attached to a `finished` event. Optional
/// per-trigger fields (fetched / new / resolved / bytes) are `None`
/// when the trigger doesn't produce them.
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobResult {
    pub triggered: bool,
    pub skipped: bool,
    pub fetched: Option<i64>,
    pub new: Option<i64>,
    pub resolved: Option<i64>,
    pub bytes: Option<i64>,
}

/// Single broadcast frame for the SSE channel. Always has `kind`, `id`,
/// `phase`, and `at` (epoch millis); `result` is only populated on
/// `Finished` events.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobEvent {
    pub kind: JobKind,
    pub id: String,
    pub phase: JobPhase,
    pub at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<JobResult>,
}

impl JobEvent {
    /// `Started` after the per-key mutex was acquired. Not emitted for
    /// `skipped=true` triggers (they go straight to a `Finished` frame).
    pub fn started(kind: JobKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            phase: JobPhase::Started,
            at: now_ms(),
            result: None,
        }
    }

    pub fn finished(kind: JobKind, id: impl Into<String>, result: JobResult) -> Self {
        Self {
            kind,
            id: id.into(),
            phase: JobPhase::Finished,
            at: now_ms(),
            result: Some(result),
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
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
    /// jobs intentionally do **not** publish here — this channel is
    /// the "user is staring at the screen" signal, not full audit log.
    pub job_events: broadcast::Sender<JobEvent>,
}

impl AppState {
    /// Best-effort publish. We don't propagate the `SendError` because
    /// the only failure mode is "no receivers connected," which is the
    /// normal state when nobody has the admin page open.
    pub fn send_job_event(&self, event: JobEvent) {
        let _ = self.job_events.send(event);
    }
}
