//! Application state shared by every handler.
//!
//! Built once in the binary's `serve` command and passed into [`crate::router`].
//! Cheap to clone (everything is `Arc`).

use std::future::Future;
use std::sync::Arc;

use sea_orm::DatabaseConnection;
use serde::Serialize;
use td_config::{AuthConfig, IngestionConfig, MetadataConfig, ProvidersConfig, SourceConfig};
use td_metadata::MetadataRegistry;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;
use td_scheduler::JobLocks;
use td_source::SourceRegistry;
use tokio::sync::{Mutex, broadcast};
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

/// Currently-running marker hung off each source / provider listing entry
/// so the admin UI can render the "RUNNING…" pill straight from the DTO
/// on a fresh page load, without waiting for the SSE channel to replay
/// state it never had.
///
/// Hydrated from the matching `*_runs` row whose `status = 'running'`.
/// Progress fields land in a later phase; for now only the start time is
/// meaningful and the pill renders the binary "is in flight" state.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InFlight {
    /// Epoch seconds the in-flight run started at.
    pub started_at: i64,
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

    /// Acquire `lock` via [`tokio::sync::Mutex::try_lock_owned`] and spawn
    /// `work` with the guard moved in, owning the full
    /// `started`/`finished`-event lifecycle for the spawned task. Returns
    /// `true` when work was dispatched, `false` when a previous holder
    /// still has the lock.
    ///
    /// Lifecycle semantics:
    /// - **Success path:** emits one [`JobEvent::started`] before the
    ///   spawn, then the spawned task awaits `work` and emits one
    ///   [`JobEvent::finished`] carrying the returned [`JobResult`].
    /// - **Contention path:** emits a single [`JobEvent::finished`] with
    ///   `triggered: false, skipped: true` and no preceding `started`,
    ///   then returns `false` without spawning.
    ///
    /// This closes the race window the open-coded
    /// `try_lock().is_err()` + drop + spawn pattern leaves open, where two
    /// near-simultaneous handler calls could both report
    /// `triggered: true` even though only one task body actually runs.
    pub fn try_dispatch<F, Fut>(
        &self,
        lock: Arc<Mutex<()>>,
        kind: JobKind,
        key: impl Into<String>,
        work: F,
    ) -> bool
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = JobResult> + Send + 'static,
    {
        try_dispatch_via(&self.job_events, lock, kind, key, work)
    }
}

/// Lock + spawn + event-emission core used by [`AppState::try_dispatch`].
/// Factored out so tests can exercise the lifecycle against a bare
/// [`broadcast::Sender`] without standing up a full [`AppState`].
pub(crate) fn try_dispatch_via<F, Fut>(
    events: &broadcast::Sender<JobEvent>,
    lock: Arc<Mutex<()>>,
    kind: JobKind,
    key: impl Into<String>,
    work: F,
) -> bool
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = JobResult> + Send + 'static,
{
    let key = key.into();
    let Ok(guard) = lock.try_lock_owned() else {
        let _ = events.send(JobEvent::finished(
            kind,
            key,
            JobResult {
                triggered: false,
                skipped: true,
                ..Default::default()
            },
        ));
        return false;
    };
    let _ = events.send(JobEvent::started(kind, key.clone()));
    let events_for_task = events.clone();
    tokio::spawn(async move {
        let _g = guard;
        let result = work().await;
        let _ = events_for_task.send(JobEvent::finished(kind, key, result));
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::oneshot;
    use tokio::time::timeout;

    /// Await exactly `expected` events off `rx`, with a per-event timeout
    /// so a missing event fails the test rather than hanging the suite.
    /// Order is whatever the broadcast channel delivered; callers assert
    /// on counts, not positions, so the spawn ordering between the
    /// helper's `started` send and the task body's `finished` send can
    /// vary without flake.
    async fn collect_events(
        rx: &mut broadcast::Receiver<JobEvent>,
        expected: usize,
    ) -> Vec<JobEvent> {
        let mut out = Vec::with_capacity(expected);
        for i in 0..expected {
            match timeout(Duration::from_secs(2), rx.recv()).await {
                Ok(Ok(event)) => out.push(event),
                Ok(Err(e)) => panic!(
                    "broadcast channel closed while collecting event {}/{expected}: {e:?}; got so far: {out:?}",
                    i + 1
                ),
                Err(_) => panic!(
                    "timed out waiting for event {}/{expected}; got so far: {out:?}",
                    i + 1
                ),
            }
        }
        out
    }

    fn count_started(events: &[JobEvent]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e.phase, JobPhase::Started))
            .count()
    }

    fn count_finished_with(events: &[JobEvent], skipped: bool) -> usize {
        events
            .iter()
            .filter(|e| matches!(e.phase, JobPhase::Finished))
            .filter(|e| {
                e.result
                    .as_ref()
                    .map(|r| r.skipped == skipped)
                    .unwrap_or(false)
            })
            .count()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn happy_path_emits_started_then_finished_with_work_result() {
        let (tx, mut rx) = broadcast::channel::<JobEvent>(16);
        let lock = Arc::new(Mutex::new(()));

        let triggered = try_dispatch_via(&tx, lock, JobKind::Provider, "mangabaka", || async {
            JobResult {
                triggered: true,
                skipped: false,
                new: Some(42),
                ..Default::default()
            }
        });
        assert!(triggered, "uncontested dispatch should win the lock");

        let events = collect_events(&mut rx, 2).await;
        let started = events
            .iter()
            .find(|e| matches!(e.phase, JobPhase::Started))
            .expect("Started present");
        assert_eq!(started.id, "mangabaka");

        let finished = events
            .iter()
            .find(|e| matches!(e.phase, JobPhase::Finished))
            .expect("Finished present");
        let finished_result = finished.result.as_ref().expect("Finished carries result");
        assert!(finished_result.triggered);
        assert!(!finished_result.skipped);
        assert_eq!(finished_result.new, Some(42));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn contended_dispatch_emits_one_skipped_finished_and_no_started() {
        let (tx, mut rx) = broadcast::channel::<JobEvent>(16);
        let lock = Arc::new(Mutex::new(()));

        // First dispatch: hold the lock until the test releases it, so the
        // second call deterministically loses the race.
        let (lock_taken_tx, lock_taken_rx) = oneshot::channel::<()>();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let first = try_dispatch_via(
            &tx,
            lock.clone(),
            JobKind::Source,
            "nyaa",
            move || async move {
                let _ = lock_taken_tx.send(());
                let _ = release_rx.await;
                JobResult {
                    triggered: true,
                    skipped: false,
                    ..Default::default()
                }
            },
        );
        assert!(first, "first dispatch should win the lock");

        // Wait until the spawned task has actually grabbed the guard, so
        // the second `try_lock_owned` is guaranteed to fail.
        timeout(Duration::from_secs(1), lock_taken_rx)
            .await
            .expect("first work should reach its body within the test budget")
            .expect("first work should signal lock acquisition");

        let second = try_dispatch_via(&tx, lock.clone(), JobKind::Source, "nyaa", || async {
            panic!("contended dispatch must not invoke work")
        });
        assert!(!second, "second dispatch should report skipped");

        // Let the first task finish so its Finished event lands too.
        let _ = release_tx.send(());

        let events = collect_events(&mut rx, 3).await;
        assert_eq!(
            count_started(&events),
            1,
            "exactly one Started expected (no phantom Started on the skipped call); got {events:?}"
        );
        assert_eq!(
            count_finished_with(&events, true),
            1,
            "exactly one Finished{{skipped:true}} expected; got {events:?}"
        );
        assert_eq!(
            count_finished_with(&events, false),
            1,
            "exactly one Finished{{skipped:false}} expected; got {events:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn distinct_locks_dispatch_independently() {
        let (tx, mut rx) = broadcast::channel::<JobEvent>(16);
        let lock_a = Arc::new(Mutex::new(()));
        let lock_b = Arc::new(Mutex::new(()));

        let a = try_dispatch_via(&tx, lock_a, JobKind::Source, "nyaa", || async {
            JobResult {
                triggered: true,
                skipped: false,
                ..Default::default()
            }
        });
        let b = try_dispatch_via(&tx, lock_b, JobKind::Source, "subsplease", || async {
            JobResult {
                triggered: true,
                skipped: false,
                ..Default::default()
            }
        });
        assert!(a && b, "two different locks should both dispatch");

        let events = collect_events(&mut rx, 4).await;
        assert_eq!(count_started(&events), 2);
        assert_eq!(count_finished_with(&events, false), 2);
        assert_eq!(count_finished_with(&events, true), 0);
    }
}
