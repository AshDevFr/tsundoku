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
//! scheduler. Operators can retry manually via
//! `tsundoku refresh-provider-cache`.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Result, anyhow};
use sea_orm::DatabaseConnection;
use td_db::repos::provider_cache_state_repo;
use td_db::repos::run_metrics_repo::{self, ProgressTable, ProviderRefreshCounts};
use td_metadata::{MetadataProvider, RefreshStatus};
use tokio::sync::broadcast;
use tokio_cron_scheduler::{Job, JobSchedulerError};

use crate::JobLocks;
use crate::error_kind;
use crate::events::{JobEvent, JobKind};
use crate::jobs::progress::ProgressHandle;

pub fn build(
    cron: &str,
    provider: Arc<dyn MetadataProvider>,
    db: DatabaseConnection,
    locks: Arc<JobLocks>,
    events: broadcast::Sender<JobEvent>,
) -> Result<Job> {
    let job = Job::new_async(cron, move |_uuid, _scheduler| {
        let provider = provider.clone();
        let db = db.clone();
        let locks = locks.clone();
        let events = events.clone();
        Box::pin(async move {
            run_tick(provider, db, locks, events, run_metrics_repo::trigger::CRON).await;
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
    events: broadcast::Sender<JobEvent>,
    trigger: &str,
) {
    let id = provider.id().to_string();
    let started_at_ts = chrono::Utc::now().timestamp();

    let lock = locks.provider_lock(&id);
    let Ok(_guard) = lock.try_lock() else {
        tracing::debug!(provider = %id, "previous refresh still running; skipping");
        record_skipped(&db, &id, started_at_ts, trigger).await;
        return;
    };

    let metrics_id = match run_metrics_repo::start_provider_refresh(
        &db,
        &id,
        started_at_ts,
        trigger,
    )
    .await
    {
        Ok(rid) => Some(rid),
        Err(e) => {
            tracing::warn!(error = ?e, provider = %id, "failed to record provider_refresh start");
            None
        }
    };

    // Live progress. The trait surface for `refresh_cache` is one async
    // call with no inner-phase callback, so we can't (yet) emit
    // download/extract/index phase transitions or byte counts — that
    // requires extending `MetadataProvider`. For now the pill renders
    // "Running... (refreshing)" so the operator at least sees a phase
    // label distinct from the binary in-flight state.
    let progress = ProgressHandle::new(
        db.clone(),
        ProgressTable::ProviderRefreshes,
        metrics_id,
        events,
        JobKind::Provider,
        &id,
    );
    progress.set_phase("refreshing").await;

    // Stopwatch the refresh call so the admin metrics view can plot dump
    // download latency over time.
    let fetch_started = Instant::now();
    let summary = match provider.refresh_cache().await {
        Ok(s) => s,
        Err(e) => {
            let ms = fetch_started.elapsed().as_millis() as i64;
            let err: anyhow::Error = anyhow::Error::new(e).context(format!("refresh {id} failed"));
            tracing::warn!(error = ?err, provider = %id, "cache refresh failed");
            let msg = format!("error: {err}");
            let kind_class = error_kind::classify_anyhow(&err);
            finalize_metrics(
                &db,
                metrics_id,
                run_metrics_repo::status::FAILURE,
                ProviderRefreshCounts {
                    fetch_duration_ms: Some(ms),
                    ..Default::default()
                },
                Some(&msg),
                Some(kind_class),
            )
            .await;
            return;
        }
    };
    let fetch_duration_ms = fetch_started.elapsed().as_millis() as i64;

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
            finalize_metrics(
                &db,
                metrics_id,
                run_metrics_repo::status::SUCCESS,
                ProviderRefreshCounts {
                    bytes_downloaded: summary.bytes_downloaded.map(|b| b as i64),
                    record_count: Some(*records as i64),
                    fetch_duration_ms: Some(fetch_duration_ms),
                },
                None,
                None,
            )
            .await;
        }
        RefreshStatus::UpToDate => {
            tracing::info!(provider = %id, "cache up to date; no refresh needed");
            // Treat "up to date" as success: the tick reached a clean
            // terminal state and the cache is usable.
            finalize_metrics(
                &db,
                metrics_id,
                run_metrics_repo::status::SUCCESS,
                ProviderRefreshCounts {
                    fetch_duration_ms: Some(fetch_duration_ms),
                    ..Default::default()
                },
                Some("up to date"),
                None,
            )
            .await;
        }
        RefreshStatus::NotSupported => {
            tracing::debug!(
                provider = %id,
                "provider has no offline cache; refresh tick is a no-op"
            );
            // Finalise with "skipped" rather than dropping the row entirely
            // so the admin metrics view shows a consistent number of ticks;
            // a "not supported" provider still consumes a scheduler slot.
            finalize_metrics(
                &db,
                metrics_id,
                run_metrics_repo::status::SKIPPED,
                ProviderRefreshCounts::default(),
                Some("provider has no offline cache"),
                None,
            )
            .await;
        }
        RefreshStatus::Skipped { message } => {
            tracing::warn!(provider = %id, %message, "cache refresh skipped");
            finalize_metrics(
                &db,
                metrics_id,
                run_metrics_repo::status::SKIPPED,
                ProviderRefreshCounts::default(),
                Some(message),
                None,
            )
            .await;
        }
    }
}

async fn record_skipped(db: &DatabaseConnection, id: &str, started_at: i64, trigger: &str) {
    match run_metrics_repo::start_provider_refresh(db, id, started_at, trigger).await {
        Ok(rid) => {
            if let Err(e) = run_metrics_repo::finalize_provider_refresh(
                db,
                rid,
                started_at,
                run_metrics_repo::status::SKIPPED,
                ProviderRefreshCounts::default(),
                Some("previous refresh still running"),
                None,
            )
            .await
            {
                tracing::warn!(error = ?e, provider = %id, "failed to record skipped provider_refresh");
            }
        }
        Err(e) => {
            tracing::warn!(error = ?e, provider = %id, "failed to insert skipped provider_refresh row");
        }
    }
}

async fn finalize_metrics(
    db: &DatabaseConnection,
    id: Option<i64>,
    status: &str,
    counts: ProviderRefreshCounts,
    error_message: Option<&str>,
    error_kind: Option<&str>,
) {
    let Some(id) = id else { return };
    let finished_at = chrono::Utc::now().timestamp();
    if let Err(e) = run_metrics_repo::finalize_provider_refresh(
        db,
        id,
        finished_at,
        status,
        counts,
        error_message,
        error_kind,
    )
    .await
    {
        tracing::warn!(error = ?e, run_id = id, "failed to finalize provider_refresh row");
    }
}
