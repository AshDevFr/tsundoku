//! Live-progress handle for long-running scheduler jobs.
//!
//! A job constructs a [`ProgressHandle`] from the in-flight `*_runs` row
//! it already owns and the broadcast sender from
//! [`SchedulerContext`](crate::SchedulerContext). Inside the loop the
//! handle is poked via [`ProgressHandle::set_total`],
//! [`ProgressHandle::set_phase`], and [`ProgressHandle::tick_to`] as the
//! work advances.
//!
//! Two output channels with deliberately different cadences:
//!
//! - **SSE.** Every state-changing call emits one `JobEvent::progress`
//!   frame on the broadcast channel. The channel drops for laggy
//!   receivers (`broadcast::Sender::send` semantics), so a slow client
//!   misses intermediate frames and resyncs from the next one — that's
//!   the right shape for ephemeral progress.
//! - **DB.** `tick_to` writes to the `progress_*` columns at most once
//!   per `max(1, total / 20)` items, or once per [`DB_INTERVAL`],
//!   whichever fires first. `set_total` and `set_phase` write
//!   immediately (rare, important). [`ProgressHandle::flush`] writes
//!   whatever is pending — the wrapper calls this at job end so the
//!   final number is visible to a refresh.
//!
//! Why two cadences: the SSE channel is in-memory, basically free per
//! frame. The DB write goes through the single-writer SQLite pool, so a
//! 500-item loop emitting per-item UPDATEs would queue behind every
//! other DB operation in the process. Twenty checkpoints per job is the
//! sweet spot — enough resolution that the pill never looks stuck, few
//! enough writes that the pool isn't a bottleneck.
//!
//! Concurrency: the handle is owned by the loop and used single-threaded.
//! Internal state is wrapped in a `tokio::sync::Mutex` so the type stays
//! `Send` for tasks that need to shuffle it across `.await` points.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sea_orm::DatabaseConnection;
use td_db::repos::run_metrics_repo::{self, ProgressSnapshot, ProgressTable};
use tokio::sync::{Mutex, broadcast};

use crate::events::{JobEvent, JobKind, JobProgress};

/// Wall-clock fallback for DB checkpointing. Even if the loop ticks
/// infrequently in `current` terms (e.g. tar extracts large files
/// slowly), a checkpoint lands every couple seconds so the pill never
/// looks frozen on a refresh.
pub const DB_INTERVAL: Duration = Duration::from_secs(2);

/// Live-progress reporter. One instance per in-flight `*_runs` row.
///
/// Clone-cheap (`Arc<Inner>`) so the same handle can be passed through
/// closures or shared between an outer driver loop and an inner async
/// helper.
#[derive(Clone)]
pub struct ProgressHandle {
    inner: Arc<Inner>,
}

struct Inner {
    db: DatabaseConnection,
    table: ProgressTable,
    row_id: Option<i64>,
    sender: broadcast::Sender<JobEvent>,
    kind: JobKind,
    key: String,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    total: Option<u64>,
    current: u64,
    phase: Option<String>,
    last_db_write: Instant,
    last_db_current: u64,
    last_db_total: Option<u64>,
    last_db_phase: Option<String>,
}

impl ProgressHandle {
    /// Build a handle. `row_id = None` constructs a no-op handle — useful
    /// when the upstream `start_*_run` insert failed and the job decided
    /// to soldier on anyway; the SSE channel still gets `Progress` frames,
    /// just no DB writes.
    pub fn new(
        db: DatabaseConnection,
        table: ProgressTable,
        row_id: Option<i64>,
        sender: broadcast::Sender<JobEvent>,
        kind: JobKind,
        key: impl Into<String>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                table,
                row_id,
                sender,
                kind,
                key: key.into(),
                state: Mutex::new(State {
                    total: None,
                    current: 0,
                    phase: None,
                    last_db_write: Instant::now(),
                    last_db_current: 0,
                    last_db_total: None,
                    last_db_phase: None,
                }),
            }),
        }
    }

    /// Set the upper bound. Writes to the DB immediately (rare event;
    /// no point throttling) and emits an SSE frame so a watching client
    /// can render the fraction denominator straight away.
    pub async fn set_total(&self, total: u64) {
        let snapshot = {
            let mut st = self.inner.state.lock().await;
            st.total = Some(total);
            self.snapshot_locked(&st)
        };
        self.write_db(&snapshot).await;
        self.emit_sse(&snapshot);
    }

    /// Set the phase label for multi-stage jobs (`"downloading"`,
    /// `"extracting"`, ...). Immediate DB write; phase transitions are
    /// load-bearing UX cues so the pill should reflect them on the next
    /// refresh.
    pub async fn set_phase(&self, phase: impl Into<String>) {
        let phase = phase.into();
        let snapshot = {
            let mut st = self.inner.state.lock().await;
            st.phase = Some(phase);
            self.snapshot_locked(&st)
        };
        self.write_db(&snapshot).await;
        self.emit_sse(&snapshot);
    }

    /// Bump the `current` counter. Always emits an SSE frame. Writes to
    /// the DB only when one of:
    ///
    /// - `current` has advanced by `>= max(1, total / 20)` since the
    ///   last DB write,
    /// - or [`DB_INTERVAL`] has elapsed since the last DB write.
    ///
    /// The throttling means a 500-item batch with `total = 500` triggers
    /// ~20 writes (every 25 items); a 50-item batch triggers ~50 (one
    /// per item, since `50/20 = 2` and the threshold is rounded down via
    /// `max(1, ...)`). Both within the budget.
    pub async fn tick_to(&self, current: u64) {
        let (snapshot, should_write_db) = {
            let mut st = self.inner.state.lock().await;
            st.current = current;
            let snapshot = self.snapshot_locked(&st);
            let should_write = self.should_checkpoint_locked(&st);
            (snapshot, should_write)
        };
        if should_write_db {
            self.write_db(&snapshot).await;
        }
        self.emit_sse(&snapshot);
    }

    /// Flush whatever is pending to the DB. Called by the job wrapper
    /// right before `finalize_*_run` so the final-tick value is visible
    /// to a refresh even if it landed mid-throttle-window.
    pub async fn flush(&self) {
        let snapshot = {
            let st = self.inner.state.lock().await;
            self.snapshot_locked(&st)
        };
        self.write_db(&snapshot).await;
    }

    fn snapshot_locked(&self, st: &State) -> ProgressSnapshot {
        ProgressSnapshot {
            current: st.current as i64,
            total: st.total.map(|t| t as i64),
            phase: st.phase.clone(),
        }
    }

    fn should_checkpoint_locked(&self, st: &State) -> bool {
        let item_threshold = st.total.map(|t| t / 20).unwrap_or(0).max(1);
        let advanced = st.current.saturating_sub(st.last_db_current) >= item_threshold;
        let elapsed = st.last_db_write.elapsed() >= DB_INTERVAL;
        advanced || elapsed
    }

    async fn write_db(&self, snapshot: &ProgressSnapshot) {
        let Some(row_id) = self.inner.row_id else {
            return;
        };
        if let Err(e) =
            run_metrics_repo::record_progress(&self.inner.db, self.inner.table, row_id, snapshot)
                .await
        {
            tracing::warn!(
                error = ?e,
                kind = ?self.inner.kind,
                key = %self.inner.key,
                "progress checkpoint write failed; SSE frames still emitted"
            );
            return;
        }
        let mut st = self.inner.state.lock().await;
        st.last_db_write = Instant::now();
        st.last_db_current = snapshot.current as u64;
        if snapshot.total.is_some() {
            st.last_db_total = snapshot.total.map(|t| t as u64);
        }
        if snapshot.phase.is_some() {
            st.last_db_phase = snapshot.phase.clone();
        }
    }

    fn emit_sse(&self, snapshot: &ProgressSnapshot) {
        let _ = self.inner.sender.send(JobEvent::progress(
            self.inner.kind,
            self.inner.key.clone(),
            JobProgress {
                current: snapshot.current as u64,
                total: snapshot.total.map(|t| t as u64),
                phase: snapshot.phase.clone(),
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::Database;
    use sea_orm::EntityTrait;
    use td_db::entities::poll_runs;

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    async fn seed_poll_row(db: &DatabaseConnection) -> i64 {
        run_metrics_repo::start_poll_run(db, "feed-a", "nyaa", 100, "manual")
            .await
            .unwrap()
    }

    async fn current_row(db: &DatabaseConnection, id: i64) -> poll_runs::Model {
        poll_runs::Entity::find_by_id(id)
            .one(db)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn tick_to_emits_sse_every_call() {
        let db = fresh_db().await;
        let id = seed_poll_row(&db).await;
        let (tx, mut rx) = broadcast::channel(64);
        let handle = ProgressHandle::new(
            db,
            ProgressTable::PollRuns,
            Some(id),
            tx,
            JobKind::Source,
            "feed-a",
        );
        handle.set_total(75).await;
        // set_total emits one SSE frame, plus one DB write.
        rx.recv().await.unwrap();

        for i in 1..=10 {
            handle.tick_to(i).await;
        }
        // 10 SSE frames expected, one per tick.
        for _ in 0..10 {
            let evt = rx.recv().await.unwrap();
            assert!(matches!(evt.phase, crate::events::JobPhase::Progress));
            assert!(evt.progress.is_some());
        }
    }

    #[tokio::test]
    async fn tick_to_throttles_db_writes_by_item_threshold() {
        let db = fresh_db().await;
        let id = seed_poll_row(&db).await;
        let (tx, _rx) = broadcast::channel(64);
        let handle = ProgressHandle::new(
            db.clone(),
            ProgressTable::PollRuns,
            Some(id),
            tx,
            JobKind::Source,
            "feed-a",
        );
        // total = 400 → item_threshold = 20. The very first tick after a
        // set_total is allowed to skip the DB write because last_db_current
        // was bumped by set_total (to 0). Writes land at current >= 20.
        handle.set_total(400).await;
        for i in 1..20 {
            handle.tick_to(i).await;
        }
        let row = current_row(&db, id).await;
        // set_total wrote 0; intermediate ticks did not.
        assert_eq!(row.progress_current, Some(0));

        handle.tick_to(20).await;
        let row = current_row(&db, id).await;
        assert_eq!(row.progress_current, Some(20));

        // No more writes until the next threshold.
        for i in 21..40 {
            handle.tick_to(i).await;
        }
        let row = current_row(&db, id).await;
        assert_eq!(row.progress_current, Some(20));

        handle.tick_to(40).await;
        let row = current_row(&db, id).await;
        assert_eq!(row.progress_current, Some(40));
    }

    #[tokio::test]
    async fn small_total_uses_one_item_threshold() {
        // total / 20 = 0 for totals < 20; max(1, _) means every tick
        // becomes a DB write. Acceptable: small batches are cheap.
        let db = fresh_db().await;
        let id = seed_poll_row(&db).await;
        let (tx, _rx) = broadcast::channel(64);
        let handle = ProgressHandle::new(
            db.clone(),
            ProgressTable::PollRuns,
            Some(id),
            tx,
            JobKind::Source,
            "feed-a",
        );
        handle.set_total(5).await;
        for i in 1..=5 {
            handle.tick_to(i).await;
        }
        let row = current_row(&db, id).await;
        assert_eq!(row.progress_current, Some(5));
    }

    #[tokio::test]
    async fn set_phase_writes_immediately() {
        let db = fresh_db().await;
        let id = seed_poll_row(&db).await;
        let (tx, _rx) = broadcast::channel(64);
        let handle = ProgressHandle::new(
            db.clone(),
            ProgressTable::PollRuns,
            Some(id),
            tx,
            JobKind::Source,
            "feed-a",
        );
        handle.set_phase("downloading").await;
        let row = current_row(&db, id).await;
        assert_eq!(row.progress_phase.as_deref(), Some("downloading"));
    }

    #[tokio::test]
    async fn flush_writes_pending_state() {
        let db = fresh_db().await;
        let id = seed_poll_row(&db).await;
        let (tx, _rx) = broadcast::channel(64);
        let handle = ProgressHandle::new(
            db.clone(),
            ProgressTable::PollRuns,
            Some(id),
            tx,
            JobKind::Source,
            "feed-a",
        );
        // total=400 ⇒ item_threshold=20. Ticking to 15 won't write.
        handle.set_total(400).await;
        handle.tick_to(15).await;
        let row = current_row(&db, id).await;
        assert_eq!(row.progress_current, Some(0));

        handle.flush().await;
        let row = current_row(&db, id).await;
        assert_eq!(row.progress_current, Some(15));
    }

    #[tokio::test]
    async fn elapsed_interval_forces_db_write() {
        let db = fresh_db().await;
        let id = seed_poll_row(&db).await;
        let (tx, _rx) = broadcast::channel(64);
        let handle = ProgressHandle::new(
            db.clone(),
            ProgressTable::PollRuns,
            Some(id),
            tx,
            JobKind::Source,
            "feed-a",
        );
        handle.set_total(10_000).await;
        // Below the item threshold (500). Force the timer past
        // DB_INTERVAL by mutating the inner state directly.
        handle.tick_to(7).await;
        let row = current_row(&db, id).await;
        assert_eq!(row.progress_current, Some(0));

        {
            let mut st = handle.inner.state.lock().await;
            st.last_db_write = Instant::now() - DB_INTERVAL - Duration::from_millis(50);
        }
        handle.tick_to(8).await;
        let row = current_row(&db, id).await;
        assert_eq!(
            row.progress_current,
            Some(8),
            "after DB_INTERVAL elapses, the next tick must checkpoint regardless of item count"
        );
    }

    #[tokio::test]
    async fn no_row_id_skips_db_but_still_emits_sse() {
        let db = fresh_db().await;
        let (tx, mut rx) = broadcast::channel(8);
        let handle = ProgressHandle::new(
            db,
            ProgressTable::PollRuns,
            None,
            tx,
            JobKind::Source,
            "feed-a",
        );
        handle.set_total(10).await;
        handle.tick_to(3).await;
        rx.recv().await.unwrap(); // set_total
        let evt = rx.recv().await.unwrap();
        assert!(evt.progress.is_some());
        assert_eq!(evt.progress.unwrap().current, 3);
    }
}
