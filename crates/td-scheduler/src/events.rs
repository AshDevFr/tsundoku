//! Job-lifecycle event types broadcast to the SSE channel.
//!
//! These describe what a job is doing — they're scheduler-shaped, not
//! HTTP-shaped, so they live here rather than in `td-api`. The api crate
//! re-exports them for handler/test convenience and registers them in
//! the OpenAPI schema list.
//!
//! Two producers feed the same `broadcast::Sender<JobEvent>`:
//!
//! - Manual-trigger handlers in `td-api` go through `AppState::try_dispatch`,
//!   which emits one `Started` (or one `Finished{skipped}`) and the spawned
//!   work emits one `Finished` at the end.
//! - Cron-driven jobs in this crate emit `Progress` frames via
//!   [`ProgressHandle`](crate::jobs::progress::ProgressHandle) and (with
//!   future work) optionally `Started`/`Finished` too.
//!
//! SSE has no replay: a client that connects mid-job won't see the
//! `Started` frame. That's intentional — the in-flight pill is hydrated
//! from `td-api::InFlight` on the listing DTO instead.

use serde::Serialize;
use utoipa::ToSchema;

/// Bounded buffer for the manual-trigger / progress event channel. Generous
/// enough that a slow client never causes the producer to lag, small enough
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
    /// Codex presence sync (sweep of Codex's series external-index).
    Codex,
    /// Per-series release search against a `[[search]]` entry.
    Search,
    /// Bulk release re-enrich across origins (the per-origin groups inside
    /// the run still emit their progress as [`JobKind::Source`] frames).
    Reenrich,
}

/// Lifecycle phase. `Started` fires after the per-key mutex was
/// acquired (so a `skipped` job emits only `Finished`). `Progress`
/// frames fire from inside the work body via
/// [`ProgressHandle`](crate::jobs::progress::ProgressHandle); they are
/// not throttled the way DB progress writes are — the broadcast channel
/// back-pressure handles laggy consumers by dropping intermediate frames.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum JobPhase {
    Started,
    Progress,
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

/// Progress payload attached to a `Progress` SSE frame and to the
/// `InFlight` DTO when the in-flight row carries a last-checkpoint
/// snapshot.
///
/// `total` is optional because some phases of some jobs don't have a
/// meaningful upper bound (e.g. the tar-extract phase of the provider
/// cache refresh, which streams files of unknown count); a `current`-only
/// payload still lets the UI render "doing N units" with no fraction.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub current: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Free-text label for multi-stage jobs. Provider cache refresh uses
    /// it for `"downloading"` / `"extracting"` / `"indexing"`; other jobs
    /// leave it `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

/// Single broadcast frame for the SSE channel. Always has `kind`, `id`,
/// `phase`, and `at` (epoch millis); `result` is only populated on
/// `Finished` events; `progress` is only populated on `Progress` events.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobEvent {
    pub kind: JobKind,
    pub id: String,
    pub phase: JobPhase,
    pub at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<JobResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<JobProgress>,
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
            progress: None,
        }
    }

    pub fn finished(kind: JobKind, id: impl Into<String>, result: JobResult) -> Self {
        Self {
            kind,
            id: id.into(),
            phase: JobPhase::Finished,
            at: now_ms(),
            result: Some(result),
            progress: None,
        }
    }

    /// `Progress` frame emitted by
    /// [`ProgressHandle`](crate::jobs::progress::ProgressHandle) as a job
    /// advances. Not throttled at this layer — the broadcast channel
    /// drops for laggy receivers, which is the right shape for
    /// ephemeral progress updates.
    pub fn progress(kind: JobKind, id: impl Into<String>, progress: JobProgress) -> Self {
        Self {
            kind,
            id: id.into(),
            phase: JobPhase::Progress,
            at: now_ms(),
            result: None,
            progress: Some(progress),
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
