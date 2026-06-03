//! Background download-client connection re-test job.
//!
//! Registered only when `[download]` is enabled, a client was built, and
//! `download.health_cron` is set. Each tick probes the client and records the
//! result via `download_status_repo::record_check` (which appends a history row
//! only on a reachability transition). Unlike [`super::sync_codex`], this takes
//! no [`crate::JobLocks`] entry and emits no [`crate::JobEvent`]: the probe is a
//! sub-second idempotent write to a singleton row, so a tick that overlaps a
//! manual `POST /download/test` is harmless under the single-connection pool.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use sea_orm::DatabaseConnection;
use td_db::repos::{TRIGGER_CRON, download_status_repo};
use td_download::DownloadClient;
use tokio_cron_scheduler::{Job, JobSchedulerError};

pub fn build(cron: &str, client: Arc<dyn DownloadClient>, db: DatabaseConnection) -> Result<Job> {
    let job = Job::new_async(cron, move |_uuid, _scheduler| {
        let client = client.clone();
        let db = db.clone();
        Box::pin(async move {
            run_tick(client.as_ref(), &db).await;
        })
    })
    .map_err(|e: JobSchedulerError| anyhow!("building download health job: {e}"))?;
    Ok(job)
}

/// One health-probe tick. Probes the client and records the outcome under the
/// `cron` trigger.
pub async fn run_tick(client: &dyn DownloadClient, db: &DatabaseConnection) {
    let now = chrono::Utc::now().timestamp();
    let (reachable, error) = match client.test_connection().await {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };
    if let Err(e) =
        download_status_repo::record_check(db, reachable, error.as_deref(), now, TRIGGER_CRON).await
    {
        tracing::warn!(error = ?e, "failed to record download health check");
    }
}
