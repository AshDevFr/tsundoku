//! Hourly snapshot of the review queue depth.
//!
//! Writes one `review_queue_snapshots` row per tick: pending counts split
//! by status + the age of the oldest still-pending release. The admin
//! metrics view reads recent rows to render a depth-over-time line and an
//! "oldest pending" callout.
//!
//! No per-key lock: the job is read-mostly (a single tally + insert) so
//! overlapping ticks are harmless. The cost of a duplicate row is one
//! extra integer per pending tally, far cheaper than the contention
//! machinery the poll path needs.

use anyhow::{Result, anyhow};
use chrono::Utc;
use sea_orm::DatabaseConnection;
use td_db::repos::review_snapshots_repo;
use tokio_cron_scheduler::{Job, JobSchedulerError};

/// Build the hourly job. Operators can override the cron from config; the
/// default in `Scheduler::build` is "every hour at minute 5".
pub fn build(cron: &str, db: DatabaseConnection) -> Result<Job> {
    let job = Job::new_async(cron, move |_uuid, _scheduler| {
        let db = db.clone();
        Box::pin(async move {
            run_tick(db).await;
        })
    })
    .map_err(|e: JobSchedulerError| anyhow!("building snapshot-review-queue job: {e}"))?;
    Ok(job)
}

/// Single snapshot pass. Public so the integration test can drive it
/// directly without the cron scheduler.
pub async fn run_tick(db: DatabaseConnection) {
    let now = Utc::now().timestamp();
    let breakdown = match review_snapshots_repo::pending_breakdown(&db).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = ?e, "snapshot_review_queue: pending tally failed");
            return;
        }
    };
    let oldest = match review_snapshots_repo::oldest_pending_age_seconds(&db, now).await {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = ?e, "snapshot_review_queue: oldest lookup failed");
            None
        }
    };
    if let Err(e) = review_snapshots_repo::insert_snapshot(&db, now, breakdown, oldest).await {
        tracing::warn!(error = ?e, "snapshot_review_queue: insert failed");
    } else {
        tracing::debug!(
            unresolved = breakdown.unresolved,
            ambiguous = breakdown.ambiguous,
            review_pending = breakdown.review_pending,
            oldest_seconds = ?oldest,
            "review-queue snapshot captured"
        );
    }
}
