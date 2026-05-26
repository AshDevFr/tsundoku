//! Scheduled cache-refresh job for a single [`MetadataProvider`].
//!
//! Each tick:
//! 1. Acquire the per-provider mutex (skip if a previous refresh is still
//!    in flight; cache refreshes can take minutes for the MangaBaka dump).
//! 2. Call `provider.refresh_cache()`.
//! 3. On [`RefreshStatus::Refreshed`], append a `provider_cache_state`
//!    row. The other statuses (`UpToDate`, `NotSupported`, `Skipped`) are
//!    logged but don't write a row — they don't represent a fresh cache.
//!
//! Errors are logged and swallowed: a failing provider must not poison the
//! scheduler. Operators can retry manually via `tsundoku refresh-metadata`.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use sea_orm::DatabaseConnection;
use td_db::repos::provider_cache_state_repo;
use td_metadata::{MetadataProvider, RefreshStatus};
use tokio_cron_scheduler::{Job, JobSchedulerError};

use crate::JobLocks;

pub fn build(
    cron: &str,
    provider: Arc<dyn MetadataProvider>,
    db: DatabaseConnection,
    locks: Arc<JobLocks>,
) -> Result<Job> {
    let job = Job::new_async(cron, move |_uuid, _scheduler| {
        let provider = provider.clone();
        let db = db.clone();
        let locks = locks.clone();
        Box::pin(async move {
            run_tick(provider, db, locks).await;
        })
    })
    .map_err(|e: JobSchedulerError| anyhow!("building refresh-provider-cache job: {e}"))?;
    Ok(job)
}

/// One refresh tick. Public so tests (and any future manual-trigger path)
/// can drive it directly without going through the cron loop.
pub async fn run_tick(
    provider: Arc<dyn MetadataProvider>,
    db: DatabaseConnection,
    locks: Arc<JobLocks>,
) {
    let id = provider.id().to_string();

    let lock = locks.provider_lock(&id);
    let Ok(_guard) = lock.try_lock() else {
        tracing::debug!(provider = %id, "previous refresh still running; skipping");
        return;
    };

    let summary = match provider.refresh_cache().await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = ?e, provider = %id, "cache refresh failed");
            return;
        }
    };

    match &summary.status {
        RefreshStatus::Refreshed { records, version } => {
            tracing::info!(
                provider = %id,
                records,
                cache_version = ?version,
                bytes = ?summary.bytes_downloaded,
                "cache refreshed"
            );
            let finished_at = summary.finished_at.timestamp();
            if let Err(e) = provider_cache_state_repo::append(
                &db,
                &id,
                finished_at,
                version.as_deref(),
                Some(*records as i64),
                None,
                summary.bytes_downloaded.map(|b| b as i64),
            )
            .await
            {
                tracing::warn!(
                    error = ?e,
                    provider = %id,
                    "failed to append provider_cache_state row"
                );
            }
        }
        RefreshStatus::UpToDate => {
            tracing::info!(provider = %id, "cache up to date; no refresh needed");
        }
        RefreshStatus::NotSupported => {
            tracing::debug!(
                provider = %id,
                "provider has no offline cache; refresh tick is a no-op"
            );
        }
        RefreshStatus::Skipped { message } => {
            tracing::warn!(provider = %id, %message, "cache refresh skipped");
        }
    }
}
