//! Scheduled + on-demand refresh of existing `series` rows against the
//! active [`MetadataProvider`].
//!
//! Each tick:
//! 1. Pull the next `batch_size` stale rows (oldest first, mapped to the
//!    active provider, manual rows excluded) via
//!    [`td_db::repos::series_refresh_repo::select_stale_for_active_provider`].
//! 2. For each row, call `provider.get(external_id)`:
//!    - `Ok(Some(meta))` → `persist::upsert_series_from_metadata` with
//!      `allow_manual_overwrite = false`. Hash-match → `unchanged`,
//!      otherwise → `refreshed`.
//!    - `Ok(None)` → bump `metadata_fetched_at` so the row rotates out
//!      of the next batch; counts as `not_found`. The series is not
//!      deleted (manual cleanup is the operator's call).
//!    - `Err(_)` → record the error, abort the batch. Burning through
//!      `batch_size` outbound calls when the provider is dead serves
//!      no one.
//! 3. Finalise the `series_refresh_runs` row with the per-outcome counts
//!    and a total `fetch_duration_ms`.
//!
//! Per-provider contention (overlapping cron fires + manual triggers) is
//! handled one layer up by [`crate::dispatch::try_dispatch`]; this body
//! does not acquire the per-provider series-refresh lock itself.
//!
//! Failures are swallowed inside the tick (logged + recorded in
//! `series_refresh_runs`); the scheduler never panics.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Result, anyhow};
use chrono::Utc;
use sea_orm::DatabaseConnection;
use td_db::repos::run_metrics_repo::{self, ProgressTable, SeriesRefreshCounts};
use td_db::repos::series_refresh_repo;
use td_metadata::MetadataProvider;
use td_resolution::persist;
use tokio::sync::broadcast;
use tokio_cron_scheduler::{Job, JobSchedulerError};

use crate::JobLocks;
use crate::dispatch;
use crate::error_kind;
use crate::events::{JobEvent, JobKind, JobResult};
use crate::jobs::progress::ProgressHandle;

#[allow(clippy::too_many_arguments)]
pub fn build(
    cron: &str,
    provider: Arc<dyn MetadataProvider>,
    db: DatabaseConnection,
    locks: Arc<JobLocks>,
    batch_size: u32,
    min_age_seconds: i64,
    events: broadcast::Sender<JobEvent>,
) -> Result<Job> {
    let job = Job::new_async(cron, move |_uuid, _scheduler| {
        let provider = provider.clone();
        let db = db.clone();
        let locks = locks.clone();
        let events = events.clone();
        Box::pin(async move {
            let provider_id = provider.id().to_string();
            let lock = locks.series_refresh_lock(&provider_id);
            let started_at_ts = Utc::now().timestamp();
            let trigger = run_metrics_repo::trigger::CRON;
            let db_for_skip = db.clone();
            let id_for_skip = provider_id.clone();
            let events_for_work = events.clone();
            dispatch::try_dispatch(
                &events,
                lock,
                JobKind::SeriesRefresh,
                provider_id.clone(),
                move || async move {
                    record_skipped(&db_for_skip, &id_for_skip, started_at_ts, trigger).await;
                },
                move || async move {
                    run_tick(
                        provider,
                        db,
                        batch_size,
                        min_age_seconds,
                        events_for_work,
                        trigger,
                    )
                    .await;
                    JobResult {
                        triggered: true,
                        skipped: false,
                        ..Default::default()
                    }
                },
            );
        })
    })
    .map_err(|e: JobSchedulerError| anyhow!("building refresh-series-metadata job: {e}"))?;
    Ok(job)
}

/// One refresh tick. Public so tests, CLI subcommands, and the manual
/// API trigger can drive it directly. Callers handle contention via
/// [`crate::dispatch::try_dispatch`]; `run_tick` does NOT acquire the
/// per-provider series-refresh lock itself.
#[allow(clippy::too_many_arguments)]
pub async fn run_tick(
    provider: Arc<dyn MetadataProvider>,
    db: DatabaseConnection,
    batch_size: u32,
    min_age_seconds: i64,
    events: broadcast::Sender<JobEvent>,
    trigger: &str,
) {
    let provider_id = provider.id().to_string();
    let started_at_ts = Utc::now().timestamp();

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

    // Live progress. Built after the batch select so set_total carries
    // the actual row count (the config batch_size is the upper bound,
    // not the realized work).
    let progress = ProgressHandle::new(
        db.clone(),
        ProgressTable::SeriesRefreshRuns,
        metrics_id,
        events,
        JobKind::SeriesRefresh,
        &provider_id,
    );
    progress.set_total(considered as u64).await;

    let mut tally = Tally::default();
    let fetch_started = Instant::now();

    for (idx, row) in batch.iter().enumerate() {
        let keep_going = process_row(&provider, &db, &provider_id, row, false, &mut tally).await;
        progress.tick_to((idx + 1) as u64).await;
        if !keep_going {
            break;
        }
    }
    progress.flush().await;

    let fetch_duration_ms = fetch_started.elapsed().as_millis() as i64;
    finalize_tally(
        &db,
        metrics_id,
        considered,
        fetch_duration_ms,
        &tally,
        "series-refresh tick complete",
        &provider_id,
    )
    .await;
}

/// Running per-row counts for one refresh pass. Shared by [`run_tick`] and
/// [`run_drain`] so both surfaces classify outcomes identically.
#[derive(Default)]
struct Tally {
    refreshed: i32,
    unchanged: i32,
    not_found: i32,
    errored: i32,
    error_message: Option<String>,
    error_kind_class: Option<&'static str>,
}

/// Fetch + persist a single stale row, updating `tally`. Returns `true` to
/// continue the batch, `false` when a provider error means the caller must
/// abort (burning the rest of the batch against a dead provider helps no
/// one).
///
/// `force_bump` is the drain switch. A single bounded tick relies on the
/// `min_age` floor to keep hash-unchanged rows out of the next batch, so it
/// leaves their `metadata_fetched_at` alone (`force_bump = false`). A drain
/// runs against a fixed cutoff with no floor, so any row persist leaves
/// untouched (hash-unchanged, or a persist error) must have its
/// `metadata_fetched_at` advanced past the cutoff here, or the drain would
/// re-select it forever.
async fn process_row(
    provider: &Arc<dyn MetadataProvider>,
    db: &DatabaseConnection,
    provider_id: &str,
    row: &series_refresh_repo::StaleSeriesRow,
    force_bump: bool,
    tally: &mut Tally,
) -> bool {
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
                db,
                provider_id,
                &meta,
                row.metadata_fetched_at,
                now,
                false,
            )
            .await
            {
                Ok(result) => {
                    if result.unchanged {
                        tally.unchanged += 1;
                        // Hash matched → persist skipped the UPDATE, so the
                        // row's metadata_fetched_at is unchanged. A drain
                        // must advance it or this row reappears every batch.
                        if force_bump {
                            bump_fetched_at(db, row.series_id).await;
                        }
                    } else {
                        tally.refreshed += 1;
                    }
                }
                Err(e) => {
                    // A persist error doesn't poison the loop: count it as
                    // errored so the metrics row reflects reality. In a
                    // single tick the row stays eligible for next time; in a
                    // drain we still bump it so the loop can't livelock on a
                    // row that keeps failing (its hash is untouched, so the
                    // next scheduled tick retries it anyway).
                    tally.errored += 1;
                    tracing::warn!(
                        error = ?e,
                        series_id = row.series_id,
                        external_id = %row.external_id,
                        "persist failed during series refresh; continuing"
                    );
                    if tally.error_message.is_none() {
                        tally.error_message =
                            Some(format!("persist failed for series {}: {e}", row.series_id));
                        tally.error_kind_class = Some(error_kind::classify_anyhow(&e));
                    }
                    if force_bump {
                        bump_fetched_at(db, row.series_id).await;
                    }
                }
            }
            true
        }
        Ok(None) => {
            tally.not_found += 1;
            bump_fetched_at(db, row.series_id).await;
            tracing::debug!(
                series_id = row.series_id,
                external_id = %row.external_id,
                "provider returned Ok(None); rotating row out of next batch"
            );
            true
        }
        Err(e) => {
            tally.errored += 1;
            let err: anyhow::Error =
                anyhow::Error::new(e).context(format!("series refresh for {provider_id}"));
            tracing::warn!(
                error = ?err,
                provider = %provider_id,
                series_id = row.series_id,
                external_id = %row.external_id,
                "provider.get failed; aborting batch"
            );
            tally.error_message = Some(format!("error: {err}"));
            tally.error_kind_class = Some(error_kind::classify_anyhow(&err));
            false
        }
    }
}

/// Advance a single row's `metadata_fetched_at` to "now". Used after
/// `Ok(None)` (rotate the vanished row out) and, in drain mode, after any
/// row persist left untouched. Failures are logged, not fatal.
async fn bump_fetched_at(db: &DatabaseConnection, series_id: i32) {
    let ts = Utc::now().timestamp();
    if let Err(e) = series_refresh_repo::bump_metadata_fetched_at(db, series_id, ts).await {
        tracing::warn!(
            error = ?e,
            series_id,
            "failed to bump metadata_fetched_at; row may reappear in next batch"
        );
    }
}

/// Drain refresh: re-fetch *every* eligible (non-manual, provider-mapped)
/// series, ignoring `min_age`, in repeated `batch_size` chunks until none
/// remain. Public so the manual API trigger and CLI can drive it; callers
/// handle contention via [`crate::dispatch::try_dispatch`].
///
/// Termination rests on a cutoff fixed at drain start: every row the loop
/// touches gets its `metadata_fetched_at` advanced past that cutoff (by
/// persist on a real change, by [`process_row`]'s `force_bump` otherwise),
/// so the eligible set strictly shrinks each batch. Concurrent inserts
/// land with `metadata_fetched_at >= cutoff` and are never picked up. A
/// provider error aborts the whole drain (same rationale as the single
/// tick). One `series_refresh_runs` row is written for the entire drain.
pub async fn run_drain(
    provider: Arc<dyn MetadataProvider>,
    db: DatabaseConnection,
    batch_size: u32,
    events: broadcast::Sender<JobEvent>,
    trigger: &str,
) {
    let provider_id = provider.id().to_string();
    let started_at_ts = Utc::now().timestamp();
    // Fixed selection cutoff for the whole drain. Touched rows advance past
    // it; everything else is left as-is.
    let cutoff = started_at_ts;
    // An explicit "refresh everything" must still work when the operator
    // set batch_size = 0 to park the scheduled tick; fall back to a sane
    // chunk so the drain paces its outbound calls.
    let chunk = if batch_size == 0 {
        DRAIN_FALLBACK_CHUNK
    } else {
        batch_size
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

    let total = series_refresh_repo::count_stale_for_active_provider(&db, &provider_id, cutoff)
        .await
        .unwrap_or(0);

    tracing::info!(
        provider = %provider_id,
        total,
        chunk,
        "series-refresh drain: walking every stale row"
    );

    let progress = ProgressHandle::new(
        db.clone(),
        ProgressTable::SeriesRefreshRuns,
        metrics_id,
        events,
        JobKind::SeriesRefresh,
        &provider_id,
    );
    progress.set_total(total).await;

    let mut tally = Tally::default();
    let mut considered = 0i32;
    let mut done = 0u64;
    let mut aborted = false;
    let fetch_started = Instant::now();

    // Backstop against a row that refuses to advance (e.g. repeated bump
    // failures): each successful batch clears `chunk` rows from the
    // eligible set, so the drain can never need more than this many passes.
    let max_batches = total / chunk as u64 + 2;
    let mut batches = 0u64;

    loop {
        if batches >= max_batches {
            tracing::warn!(
                provider = %provider_id,
                batches,
                "series-refresh drain hit its batch ceiling; stopping to avoid a livelock"
            );
            if tally.error_message.is_none() {
                tally.errored += 1;
                tally.error_message =
                    Some("drain did not converge within its batch ceiling".into());
            }
            break;
        }
        batches += 1;

        let batch = match series_refresh_repo::select_stale_for_active_provider(
            &db,
            &provider_id,
            chunk,
            0,
            cutoff,
        )
        .await
        {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    provider = %provider_id,
                    "failed to select stale series during drain"
                );
                tally.errored += 1;
                if tally.error_message.is_none() {
                    tally.error_message = Some(format!("error selecting stale series: {e}"));
                    tally.error_kind_class = Some(error_kind::classify_anyhow(&e));
                }
                break;
            }
        };
        if batch.is_empty() {
            break;
        }

        for row in &batch {
            let keep_going = process_row(&provider, &db, &provider_id, row, true, &mut tally).await;
            considered += 1;
            done += 1;
            progress.tick_to(done).await;
            if !keep_going {
                aborted = true;
                break;
            }
        }
        if aborted {
            break;
        }
    }
    progress.flush().await;

    let fetch_duration_ms = fetch_started.elapsed().as_millis() as i64;
    finalize_tally(
        &db,
        metrics_id,
        considered,
        fetch_duration_ms,
        &tally,
        "series-refresh drain complete",
        &provider_id,
    )
    .await;
}

/// Default per-batch chunk for a drain when `batch_size = 0` (scheduled
/// tick parked but the operator clicked "refresh everything").
const DRAIN_FALLBACK_CHUNK: u32 = 100;

/// Write the final `series_refresh_runs` row from an accumulated [`Tally`].
async fn finalize_tally(
    db: &DatabaseConnection,
    metrics_id: Option<i64>,
    considered: i32,
    fetch_duration_ms: i64,
    tally: &Tally,
    log_msg: &str,
    provider_id: &str,
) {
    let status = if tally.errored == 0 {
        run_metrics_repo::status::SUCCESS
    } else {
        run_metrics_repo::status::FAILURE
    };

    tracing::info!(
        provider = %provider_id,
        considered,
        refreshed = tally.refreshed,
        unchanged = tally.unchanged,
        not_found = tally.not_found,
        errored = tally.errored,
        fetch_duration_ms,
        "{log_msg}"
    );

    finalize_metrics(
        db,
        metrics_id,
        status,
        SeriesRefreshCounts {
            considered: Some(considered),
            refreshed: Some(tally.refreshed),
            unchanged: Some(tally.unchanged),
            not_found: Some(tally.not_found),
            errored: Some(tally.errored),
            fetch_duration_ms: Some(fetch_duration_ms),
        },
        tally.error_message.as_deref(),
        tally.error_kind_class,
    )
    .await;
}

/// Write a `series_refresh_runs` row marked `skipped` so the admin
/// metrics view surfaces contention. Called by
/// [`crate::dispatch::try_dispatch`] when a tick lost the per-provider
/// series-refresh lock.
pub async fn record_skipped(db: &DatabaseConnection, id: &str, started_at: i64, trigger: &str) {
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
