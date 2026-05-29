//! Re-enrich already-persisted releases for a single [`DiscoverySource`].
//!
//! Selects the source's releases whose `resolution_status` is in a
//! caller-supplied set, re-runs [`DiscoverySource::enrich`] (which, for a
//! detail-fetching source, re-pulls the post page), and re-persists each via
//! `persist_discovered`. The upsert refreshes the source-derived columns
//! (files, description, extracted links, information URL) while leaving every
//! resolution column untouched — so re-enriching a `resolved` row backfills
//! its details without un-resolving it.
//!
//! Deliberately does **not** call the resolver: re-enrich backfills detail
//! fields after a parser change, it does not re-decide matches. That is what
//! the retry / retry-all paths are for.
//!
//! Borrows the per-source `poll_runs` lane for its in-flight pill + progress,
//! the same way [`super::backfill_source`] does. Callers enforce per-source
//! contention via [`crate::dispatch::try_dispatch`]; `run` does not acquire
//! the lock itself.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use chrono::Utc;
use sea_orm::DatabaseConnection;
use td_db::repos::releases_repo;
use td_db::repos::run_metrics_repo::{self, PollRunCounts, ProgressTable};
use td_source::DiscoverySource;
use tokio::sync::broadcast;

use crate::events::{JobEvent, JobKind};
use crate::jobs::progress::ProgressHandle;

/// Upper bound on rows touched in one re-enrich run. A guard against an
/// accidental "re-enrich every resolved release" turning into tens of
/// thousands of detail-page fetches in a single tick. When the cap is hit the
/// run logs how many rows it left behind; re-trigger to continue (the walk is
/// newest-first and idempotent).
pub const REENRICH_CAP: u64 = 5_000;

/// Per-run tallies, surfaced by the SSE progress event and the run log.
#[derive(Debug, Clone, Default)]
pub struct ReenrichSummary {
    /// Rows selected for the run (after the [`REENRICH_CAP`] clamp).
    pub considered: usize,
    /// Rows whose detail page was re-fetched and re-persisted.
    pub reenriched: usize,
    /// Per-release enrich/persist failures (logged individually, non-fatal).
    pub errors: usize,
}

/// Re-enrich the source's releases matching `statuses`. Errors only on a
/// setup fault (the initial DB select); per-release failures are logged and
/// counted so a flaky detail host doesn't sink the whole run.
pub async fn run(
    source: Arc<dyn DiscoverySource>,
    db: DatabaseConnection,
    statuses: Vec<String>,
    events: broadcast::Sender<JobEvent>,
    trigger: &str,
) -> Result<ReenrichSummary> {
    let name = source.name().to_string();
    let kind = source.kind().to_string();

    let rows =
        releases_repo::select_for_reenrich(&db, &kind, &name, &statuses, REENRICH_CAP).await?;
    let considered = rows.len();
    if considered as u64 == REENRICH_CAP {
        tracing::warn!(
            source = %name,
            cap = REENRICH_CAP,
            "re-enrich hit the per-run cap; some matching rows were not touched this run — re-trigger to continue"
        );
    }
    tracing::info!(
        source = %name,
        kind = %kind,
        considered,
        statuses = ?statuses,
        trigger = %trigger,
        "re-enrich starting"
    );

    // Borrow the per-source `poll_runs` lane for the in-flight pill +
    // progress checkpoint, mirroring backfill. Non-fatal if the insert
    // fails; the loop still runs and emits SSE frames only.
    let started_at_ts = Utc::now().timestamp();
    let metrics_id = match run_metrics_repo::start_poll_run(
        &db,
        &name,
        &kind,
        started_at_ts,
        trigger,
    )
    .await
    {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::warn!(error = ?e, source = %name, "failed to record re-enrich poll_run start");
            None
        }
    };
    let progress = ProgressHandle::new(
        db.clone(),
        ProgressTable::PollRuns,
        metrics_id,
        events,
        JobKind::Source,
        &name,
    );
    progress.set_total(considered as u64).await;
    progress.set_phase("re-enriching").await;

    let mut summary = ReenrichSummary {
        considered,
        ..Default::default()
    };
    let mut enrich_total_ms: u128 = 0;
    for (idx, model) in rows.into_iter().enumerate() {
        let mut release = releases_repo::model_to_discovered(&model);
        let enrich_started = Instant::now();
        let enrich_result = source.enrich(&mut release).await;
        enrich_total_ms += enrich_started.elapsed().as_millis();
        if let Err(e) = enrich_result {
            tracing::warn!(
                error = ?e,
                source = %name,
                external_id = %release.external_id,
                "re-enrich detail fetch failed; leaving row as-is"
            );
            summary.errors += 1;
            progress.tick_to((idx + 1) as u64).await;
            continue;
        }
        // persist_discovered upserts (resolution columns are excluded from
        // the on-conflict update set, so the link/status survive) and
        // re-attaches detected formats. No resolver call: re-enrich must not
        // change a release's match.
        match releases_repo::persist_discovered(&db, &release, Utc::now().timestamp()).await {
            Ok(_) => summary.reenriched += 1,
            Err(e) => {
                tracing::error!(
                    error = ?e,
                    source = %name,
                    external_id = %release.external_id,
                    "re-enrich persist failed"
                );
                summary.errors += 1;
            }
        }
        progress.tick_to((idx + 1) as u64).await;
    }
    progress.flush().await;

    let enrich_duration_ms = enrich_total_ms.min(i64::MAX as u128) as i64;
    tracing::info!(
        source = %name,
        considered = summary.considered,
        reenriched = summary.reenriched,
        errors = summary.errors,
        enrich_duration_ms,
        "re-enrich complete"
    );

    // Finalize the borrowed poll_runs row. `new`/`resolved` are zero: a
    // re-enrich discovers no rows and decides no matches. `fetched` records
    // the rows walked so the in-flight pill clears with an honest count.
    if let Some(id) = metrics_id {
        let finished_at = Utc::now().timestamp();
        if let Err(e) = run_metrics_repo::finalize_poll_run(
            &db,
            id,
            finished_at,
            run_metrics_repo::status::SUCCESS,
            PollRunCounts {
                fetched: Some(summary.considered as i32),
                new: Some(0),
                resolved: Some(0),
                fetch_duration_ms: None,
                enrich_duration_ms: Some(enrich_duration_ms),
                resolve_duration_ms: None,
                outcome_known_id: None,
                outcome_foreign_id: None,
                outcome_fuzzy: None,
                outcome_review: None,
                outcome_failed: None,
            },
            None,
            None,
        )
        .await
        {
            tracing::warn!(error = ?e, source = %name, "failed to finalize re-enrich poll_run row");
        }
    }

    Ok(summary)
}
