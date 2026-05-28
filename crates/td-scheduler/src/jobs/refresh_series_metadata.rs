//! Scheduled + on-demand refresh of existing `series` rows against the
//! active [`MetadataProvider`].
//!
//! Each tick:
//! 1. Acquire the per-provider [`JobLocks::series_refresh_lock`] (skip if
//!    a previous tick is still running). Skipped ticks still record a
//!    `series_refresh_runs` row so the admin metrics view shows the gap
//!    rather than a missing tick.
//! 2. Pull the next `batch_size` stale rows (oldest first, mapped to the
//!    active provider, manual rows excluded) via
//!    [`td_db::repos::series_refresh_repo::select_stale_for_active_provider`].
//! 3. For each row, call `provider.get(external_id)`:
//!    - `Ok(Some(meta))` → `persist::upsert_series_from_metadata` with
//!      `allow_manual_overwrite = false`. Hash-match → `unchanged`,
//!      otherwise → `refreshed`.
//!    - `Ok(None)` → bump `metadata_fetched_at` so the row rotates out
//!      of the next batch; counts as `not_found`. The series is not
//!      deleted (manual cleanup is the operator's call).
//!    - `Err(_)` → record the error, abort the batch. Burning through
//!      `batch_size` outbound calls when the provider is dead serves
//!      no one.
//! 4. Finalise the `series_refresh_runs` row with the per-outcome counts
//!    and a total `fetch_duration_ms`.
//!
//! Failures are swallowed inside the tick (logged + recorded in
//! `series_refresh_runs`); the scheduler never panics.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Result, anyhow};
use chrono::Utc;
use sea_orm::DatabaseConnection;
use td_db::repos::run_metrics_repo::{self, SeriesRefreshCounts};
use td_db::repos::series_refresh_repo;
use td_metadata::MetadataProvider;
use td_resolution::persist;
use tokio_cron_scheduler::{Job, JobSchedulerError};

use crate::JobLocks;
use crate::error_kind;

pub fn build(
    cron: &str,
    provider: Arc<dyn MetadataProvider>,
    db: DatabaseConnection,
    locks: Arc<JobLocks>,
    batch_size: u32,
    min_age_seconds: i64,
) -> Result<Job> {
    let job = Job::new_async(cron, move |_uuid, _scheduler| {
        let provider = provider.clone();
        let db = db.clone();
        let locks = locks.clone();
        Box::pin(async move {
            run_tick(
                provider,
                db,
                locks,
                batch_size,
                min_age_seconds,
                run_metrics_repo::trigger::CRON,
            )
            .await;
        })
    })
    .map_err(|e: JobSchedulerError| anyhow!("building refresh-series-metadata job: {e}"))?;
    Ok(job)
}

/// One refresh tick. Public so tests and the manual API trigger can drive
/// it directly without going through the cron loop.
pub async fn run_tick(
    provider: Arc<dyn MetadataProvider>,
    db: DatabaseConnection,
    locks: Arc<JobLocks>,
    batch_size: u32,
    min_age_seconds: i64,
    trigger: &str,
) {
    let provider_id = provider.id().to_string();
    let started_at_ts = Utc::now().timestamp();

    let lock = locks.series_refresh_lock(&provider_id);
    let Ok(_guard) = lock.try_lock() else {
        tracing::debug!(
            provider = %provider_id,
            "previous series-refresh tick still running; skipping"
        );
        record_skipped(&db, &provider_id, started_at_ts, trigger).await;
        return;
    };

    let metrics_id =
        match run_metrics_repo::start_series_refresh_run(&db, &provider_id, started_at_ts, trigger)
            .await
        {
            Ok(rid) => Some(rid),
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    provider = %provider_id,
                    "failed to record series_refresh_run start"
                );
                None
            }
        };

    // Pull the batch. A DB error here aborts the tick before we make any
    // outbound calls.
    let now_ts = Utc::now().timestamp();
    let batch = match series_refresh_repo::select_stale_for_active_provider(
        &db,
        &provider_id,
        batch_size,
        min_age_seconds,
        now_ts,
    )
    .await
    {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                error = ?e,
                provider = %provider_id,
                "failed to select stale series for refresh"
            );
            let msg = format!("error selecting stale series: {e}");
            let kind_class = error_kind::classify_anyhow(&e);
            finalize_metrics(
                &db,
                metrics_id,
                run_metrics_repo::status::FAILURE,
                SeriesRefreshCounts::default(),
                Some(&msg),
                Some(kind_class),
            )
            .await;
            return;
        }
    };
    let considered = batch.len() as i32;

    tracing::info!(
        provider = %provider_id,
        batch_size = considered,
        "series-refresh tick: walking stale rows"
    );

    let mut refreshed = 0i32;
    let mut unchanged = 0i32;
    let mut not_found = 0i32;
    let mut errored = 0i32;
    let mut error_message: Option<String> = None;
    let mut error_kind_class: Option<&'static str> = None;
    let fetch_started = Instant::now();

    for row in &batch {
        let fetch_one_started = Instant::now();
        let outcome = provider.get(&row.external_id).await;
        let elapsed_ms = fetch_one_started.elapsed().as_millis() as u64;
        tracing::debug!(
            series_id = row.series_id,
            external_id = %row.external_id,
            elapsed_ms,
            "provider.get returned"
        );
        match outcome {
            Ok(Some(meta)) => {
                let now = Utc::now();
                match persist::upsert_series_from_metadata(
                    &db,
                    &provider_id,
                    &meta,
                    row.metadata_fetched_at,
                    now,
                    false,
                )
                .await
                {
                    Ok(result) => {
                        if result.unchanged {
                            unchanged += 1;
                        } else {
                            refreshed += 1;
                        }
                    }
                    Err(e) => {
                        // A persist error doesn't poison the loop: the
                        // row's metadata_fetched_at is unchanged so it
                        // stays eligible for next tick. Count it as
                        // errored so the metrics row reflects reality.
                        errored += 1;
                        tracing::warn!(
                            error = ?e,
                            series_id = row.series_id,
                            external_id = %row.external_id,
                            "persist failed during series refresh; continuing"
                        );
                        if error_message.is_none() {
                            error_message =
                                Some(format!("persist failed for series {}: {e}", row.series_id));
                            error_kind_class = Some(error_kind::classify_anyhow(&e));
                        }
                    }
                }
            }
            Ok(None) => {
                not_found += 1;
                if let Err(e) =
                    series_refresh_repo::bump_metadata_fetched_at(&db, row.series_id, now_ts).await
                {
                    tracing::warn!(
                        error = ?e,
                        series_id = row.series_id,
                        "failed to bump metadata_fetched_at after Ok(None); row will reappear in next batch"
                    );
                }
                tracing::debug!(
                    series_id = row.series_id,
                    external_id = %row.external_id,
                    "provider returned Ok(None); rotating row out of next batch"
                );
            }
            Err(e) => {
                errored += 1;
                let err: anyhow::Error =
                    anyhow::Error::new(e).context(format!("series refresh for {provider_id}"));
                tracing::warn!(
                    error = ?err,
                    provider = %provider_id,
                    series_id = row.series_id,
                    external_id = %row.external_id,
                    "provider.get failed; aborting batch"
                );
                error_message = Some(format!("error: {err}"));
                error_kind_class = Some(error_kind::classify_anyhow(&err));
                break;
            }
        }
    }

    let fetch_duration_ms = fetch_started.elapsed().as_millis() as i64;
    let status = if errored == 0 {
        run_metrics_repo::status::SUCCESS
    } else {
        run_metrics_repo::status::FAILURE
    };

    tracing::info!(
        provider = %provider_id,
        considered,
        refreshed,
        unchanged,
        not_found,
        errored,
        fetch_duration_ms,
        "series-refresh tick complete"
    );

    finalize_metrics(
        &db,
        metrics_id,
        status,
        SeriesRefreshCounts {
            considered: Some(considered),
            refreshed: Some(refreshed),
            unchanged: Some(unchanged),
            not_found: Some(not_found),
            errored: Some(errored),
            fetch_duration_ms: Some(fetch_duration_ms),
        },
        error_message.as_deref(),
        error_kind_class,
    )
    .await;
}

async fn record_skipped(db: &DatabaseConnection, id: &str, started_at: i64, trigger: &str) {
    match run_metrics_repo::start_series_refresh_run(db, id, started_at, trigger).await {
        Ok(rid) => {
            if let Err(e) = run_metrics_repo::finalize_series_refresh_run(
                db,
                rid,
                started_at,
                run_metrics_repo::status::SKIPPED,
                SeriesRefreshCounts::default(),
                Some("previous refresh still running"),
                None,
            )
            .await
            {
                tracing::warn!(
                    error = ?e,
                    provider = %id,
                    "failed to record skipped series_refresh_run"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                error = ?e,
                provider = %id,
                "failed to insert skipped series_refresh_run row"
            );
        }
    }
}

async fn finalize_metrics(
    db: &DatabaseConnection,
    id: Option<i64>,
    status: &str,
    counts: SeriesRefreshCounts,
    error_message: Option<&str>,
    error_kind: Option<&str>,
) {
    let Some(id) = id else { return };
    let finished_at = Utc::now().timestamp();
    if let Err(e) = run_metrics_repo::finalize_series_refresh_run(
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
        tracing::warn!(error = ?e, run_id = id, "failed to finalize series_refresh_run row");
    }
}
