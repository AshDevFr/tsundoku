//! Shared historical-catch-up driver for a single [`DiscoverySource`].
//!
//! One-shot walk of a source's paginated listing (which paginates, unlike
//! the steady-state RSS poll). For each page: fetch, then per release
//! cheap-dedup → enrich → persist → resolve. Reuses the same
//! [`DiscoverySource::enrich`] + `persist_discovered` + resolver path as
//! [`super::poll_source`] so backfilled releases are indistinguishable
//! from polled ones.
//!
//! Both the `tsundoku backfill` CLI and the `POST /sources/{name}/backfill`
//! API handler call [`run`]; per-source contention against the cron poll
//! is enforced by [`crate::dispatch::try_dispatch`] (or by being a
//! separate process for the CLI). Idempotent on
//! `(source_kind, external_id)`; never touches `source_state`, so it does
//! not move the cron's ETag / last-poll markers.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Result, anyhow};
use chrono::Utc;
use sea_orm::DatabaseConnection;
use td_config::IngestionConfig;
use td_db::repos::releases_repo::{self, id_for};
use td_db::repos::run_metrics_repo::{self, PollRunCounts, ProgressTable};
use td_metadata::MetadataRegistry;
use td_resolution::Resolver;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;
use td_source::DiscoverySource;
use tokio::sync::broadcast;

use crate::events::{JobEvent, JobKind};
use crate::jobs::outcomes::OutcomeBreakdown;
use crate::jobs::progress::ProgressHandle;

/// Per-run tallies, surfaced by the CLI summary and the API's progress event.
#[derive(Debug, Clone, Default)]
pub struct BackfillSummary {
    pub pages_walked: u32,
    /// Total rows seen across every walked page (new + already-known).
    pub total: usize,
    pub new: usize,
    /// Rows skipped because the `(source_kind, external_id)` was already present.
    pub already_known: usize,
    /// Per-release persist/resolve failures (logged individually, non-fatal).
    pub errors: usize,
}

/// Reuses the per-source `poll_runs` lane: walks up to `pages` listing
/// pages, persisting and resolving every new release. Callers handle
/// per-source contention via [`crate::dispatch::try_dispatch`];
/// `run` does NOT acquire the per-source lock itself.
///
/// Errors only on setup faults the operator must see (source not
/// registered for backfill, resolver construction). Per-page and
/// per-release failures are logged and counted, not propagated, so a
/// mid-walk hiccup still preserves the work done so far.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    source: Arc<dyn DiscoverySource>,
    db: DatabaseConnection,
    metadata: Arc<MetadataRegistry>,
    ingestion: IngestionConfig,
    query_builder: Arc<QueryBuilder>,
    mangaupdates_redirector: Option<Arc<MangaUpdatesRedirector>>,
    events: broadcast::Sender<JobEvent>,
    pages: u32,
    trigger: &str,
) -> Result<BackfillSummary> {
    let pages = pages.max(1);
    let name = source.name().to_string();
    let kind = source.kind().to_string();

    // Reject non-backfillable sources up front, so the error is about
    // capability rather than runtime failure.
    if source.as_backfillable().is_none() {
        return Err(anyhow!(
            "source {name:?} (kind={kind}) does not support historical backfill"
        ));
    }

    let mut resolver =
        Resolver::new(db.clone(), metadata, ingestion).with_query_builder(query_builder);
    if let Some(r) = mangaupdates_redirector {
        resolver = resolver.with_mangaupdates_redirector(r);
    }

    let backfillable = source
        .as_backfillable()
        .expect("as_backfillable checked Some above");

    tracing::info!(source = %name, kind = %kind, pages, trigger = %trigger, "backfill starting");

    // Backfill borrows the per-source `poll_runs` lane for its progress
    // checkpoint. Insert a running row up front so the in-flight pill
    // shows "Running... X / N pages" both during the walk and (with
    // status='running' as the tell) after a crash. Failure to insert is
    // non-fatal; the loop still runs and emits SSE frames only.
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
            tracing::warn!(error = ?e, source = %name, "failed to record backfill poll_run start");
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
    progress.set_total(pages as u64).await;
    progress.set_phase("backfilling").await;

    let mut totals = BackfillSummary::default();
    let mut breakdown = OutcomeBreakdown::default();
    // Per-phase wall-clock totals. The three numbers add up to "where did
    // the backfill spend its time" and feed the same poll_runs columns the
    // poll job populates, so a single backfill row on the metrics card
    // shows the same enrich/resolve split a steady-state poll would.
    let mut listing_total_ms: u128 = 0;
    let mut enrich_total_ms: u128 = 0;
    let mut resolve_total_ms: u128 = 0;
    let mut resolve_errors = 0usize;
    for page in 1..=pages {
        let listing_started = Instant::now();
        let listing_result = backfillable.backfill_page(page).await;
        listing_total_ms += listing_started.elapsed().as_millis();
        let releases = match listing_result {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = ?e, source = %name, page, "backfill page failed; stopping");
                break;
            }
        };
        if releases.is_empty() {
            tracing::info!(source = %name, page, "no more pages; stopping");
            break;
        }
        let page_total = releases.len();
        let mut page_new = 0usize;
        let mut page_skipped = 0usize;
        let mut page_errors = 0usize;
        let mut page_enrich_ms: u128 = 0;
        let mut page_resolve_ms: u128 = 0;
        for mut release in releases {
            let id = id_for(&release.source_kind, &release.external_id);
            // Cheap dedup: if the row already exists, skip the detail
            // fetch + resolver entirely. Re-running with the same pages
            // becomes O(rows × PK-lookup) instead of O(rows × HTTP-fetch).
            match releases_repo::find_by_id(&db, &id).await {
                Ok(Some(_)) => {
                    page_skipped += 1;
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = ?e, %id, "find_by_id failed; treating as new");
                }
            }
            let enrich_started = Instant::now();
            let enrich_result = source.enrich(&mut release).await;
            let enrich_ms = enrich_started.elapsed().as_millis();
            page_enrich_ms += enrich_ms;
            if let Err(e) = enrich_result {
                tracing::warn!(
                    error = ?e,
                    source = %name,
                    external_id = %release.external_id,
                    "enrich failed; persisting with listing-only data"
                );
            }
            let persisted_id = match releases_repo::persist_discovered(
                &db,
                &release,
                Utc::now().timestamp(),
            )
            .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!(error = ?e, external_id = %release.external_id, "persist failed");
                    page_errors += 1;
                    continue;
                }
            };
            let resolve_started = Instant::now();
            let resolve_result = resolver.resolve_one(&persisted_id).await;
            let resolve_ms = resolve_started.elapsed().as_millis();
            page_resolve_ms += resolve_ms;
            match resolve_result {
                Ok(o) => breakdown.record(&o),
                Err(e) => {
                    tracing::warn!(error = ?e, release_id = %persisted_id, "resolver failed; release left unresolved");
                    page_errors += 1;
                    resolve_errors += 1;
                    breakdown.failed += 1;
                }
            }
            tracing::debug!(
                source = %name,
                release_id = %persisted_id,
                enrich_ms = enrich_ms as u64,
                resolve_ms = resolve_ms as u64,
                "release processed"
            );
            page_new += 1;
        }
        enrich_total_ms += page_enrich_ms;
        resolve_total_ms += page_resolve_ms;
        tracing::info!(
            source = %name,
            page,
            total = page_total,
            new = page_new,
            skipped = page_skipped,
            errors = page_errors,
            enrich_ms = page_enrich_ms as u64,
            resolve_ms = page_resolve_ms as u64,
            "backfill page complete"
        );
        totals.pages_walked += 1;
        totals.total += page_total;
        totals.new += page_new;
        totals.already_known += page_skipped;
        totals.errors += page_errors;
        progress.tick_to(page as u64).await;
    }
    progress.flush().await;

    let fetch_duration_ms = listing_total_ms.min(i64::MAX as u128) as i64;
    let enrich_duration_ms = enrich_total_ms.min(i64::MAX as u128) as i64;
    let resolve_duration_ms = resolve_total_ms.min(i64::MAX as u128) as i64;
    tracing::info!(
        source = %name,
        pages = totals.pages_walked,
        total = totals.total,
        new = totals.new,
        already_known = totals.already_known,
        errors = totals.errors,
        fetch_duration_ms,
        enrich_duration_ms,
        resolve_duration_ms,
        "backfill complete"
    );

    // Finalize the backfill's poll_runs row so it transitions out of
    // `running` and the in-flight pill clears. Counts and timings mirror
    // the poll path so the metrics card aggregates both kinds of run
    // without special-casing.
    if let Some(id) = metrics_id {
        let finished_at = Utc::now().timestamp();
        if let Err(e) = run_metrics_repo::finalize_poll_run(
            &db,
            id,
            finished_at,
            run_metrics_repo::status::SUCCESS,
            PollRunCounts {
                fetched: Some(totals.total as i32),
                new: Some(totals.new as i32),
                resolved: Some(totals.new.saturating_sub(resolve_errors) as i32),
                fetch_duration_ms: Some(fetch_duration_ms),
                enrich_duration_ms: Some(enrich_duration_ms),
                resolve_duration_ms: Some(resolve_duration_ms),
                outcome_known_id: Some(breakdown.known_id),
                outcome_foreign_id: Some(breakdown.foreign_id),
                outcome_fuzzy: Some(breakdown.fuzzy),
                outcome_review: Some(breakdown.review),
                outcome_failed: Some(breakdown.failed),
            },
            None,
            None,
        )
        .await
        {
            tracing::warn!(error = ?e, source = %name, "failed to finalize backfill poll_run row");
        }
    }

    Ok(totals)
}
