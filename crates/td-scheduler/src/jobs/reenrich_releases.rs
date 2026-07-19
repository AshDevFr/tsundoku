//! Bulk re-enrich of already-persisted releases, across every origin.
//!
//! Selects releases whose `resolution_status` is in a caller-supplied set —
//! optionally scoped to a set of stamped `source_name` values, optionally
//! narrowed to rows still missing detail data — re-runs `enrich` on each
//! (which, for a detail-fetching upstream, re-pulls the post page), and
//! re-persists via `persist_discovered`. The upsert refreshes the
//! source-derived columns (files, description, extracted links, information
//! URL) while leaving every resolution column untouched — so re-enriching a
//! `resolved` row backfills its details without un-resolving it.
//!
//! Enrichment only needs the release's stored `link` plus *an* enricher of
//! the same `source_kind`, not the instance that discovered the row. The
//! caller passes one enricher per kind (a `[[sources]]` instance or a
//! `[[search]]` entry — same `enrich` contract), which is what makes rows
//! whose origin was renamed or removed from config reachable again. Rows of
//! a kind with no registered enricher are counted and skipped.
//!
//! Deliberately does **not** call the resolver: re-enrich backfills detail
//! fields after a parser change, it does not re-decide matches. That is what
//! the retry / retry-all paths are for.
//!
//! The walk is grouped by stamped origin; each group borrows that name's
//! `poll_runs` lane for an audit row + progress checkpoint, the same way
//! [`super::backfill_source`] does. Callers enforce single-run contention
//! via the global re-enrich lock in [`crate::JobLocks`]; `run` does not
//! acquire locks itself. A concurrent cron poll of one of the touched
//! sources is harmless — both sides upsert fresh detail data and neither
//! touches resolution columns.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use chrono::Utc;
use sea_orm::DatabaseConnection;
use td_db::repos::releases_repo;
use td_db::repos::run_metrics_repo::{self, PollRunCounts, ProgressTable};
use td_source::{DiscoveredRelease, DiscoverySource, SearchSource, SourceResult};
use tokio::sync::broadcast;

use crate::events::{JobEvent, JobKind};
use crate::jobs::progress::ProgressHandle;

/// Upper bound on rows touched in one re-enrich run. A guard against an
/// accidental "re-enrich every resolved release" turning into tens of
/// thousands of detail-page fetches in a single tick. When the cap is hit the
/// run logs how many rows it left behind; re-trigger to continue (the walk is
/// newest-first and idempotent).
pub const REENRICH_CAP: u64 = 5_000;

/// A detail-fetching upstream for one `source_kind`. `[[sources]]` instances
/// and `[[search]]` entries share the same `enrich` contract (non-fatal,
/// fetches the release's own stored link), so either can stand in for the
/// kind as a whole.
pub enum Enricher {
    Source(Arc<dyn DiscoverySource>),
    Search(Arc<dyn SearchSource>),
}

impl Enricher {
    async fn enrich(&self, release: &mut DiscoveredRelease) -> SourceResult<()> {
        match self {
            Self::Source(s) => s.enrich(release).await,
            Self::Search(s) => s.enrich(release).await,
        }
    }
}

/// Per-run tallies, surfaced by the run log and the HTTP trigger's audit.
#[derive(Debug, Clone, Default)]
pub struct ReenrichSummary {
    /// Rows selected for the run (after the [`REENRICH_CAP`] clamp).
    pub considered: usize,
    /// Rows whose detail page was re-fetched and re-persisted.
    pub reenriched: usize,
    /// Per-release enrich/persist failures (logged individually, non-fatal).
    pub errors: usize,
    /// Rows skipped because no enricher was supplied for their
    /// `source_kind`.
    pub skipped_no_enricher: usize,
}

/// Re-enrich the releases matching `statuses`, scoped to `source_names` when
/// given (`None` = every origin, including ones no longer configured), and
/// narrowed to rows missing files or description when
/// `only_missing_details`. Errors only on a setup fault (the initial DB
/// select); per-release failures are logged and counted so a flaky detail
/// host doesn't sink the whole run.
pub async fn run(
    db: DatabaseConnection,
    enrichers: HashMap<String, Enricher>,
    statuses: Vec<String>,
    only_missing_details: bool,
    source_names: Option<Vec<String>>,
    events: broadcast::Sender<JobEvent>,
    trigger: &str,
) -> Result<ReenrichSummary> {
    let rows = releases_repo::select_for_reenrich(
        &db,
        &statuses,
        only_missing_details,
        source_names.as_deref(),
        REENRICH_CAP,
    )
    .await?;
    let considered = rows.len();
    if considered as u64 == REENRICH_CAP {
        tracing::warn!(
            cap = REENRICH_CAP,
            "re-enrich hit the per-run cap; some matching rows were not touched this run — re-trigger to continue"
        );
    }
    tracing::info!(
        considered,
        statuses = ?statuses,
        only_missing_details,
        scope = ?source_names,
        trigger = %trigger,
        "re-enrich starting"
    );

    // Group rows by stamped origin so each group can borrow that name's
    // `poll_runs` lane. Groups keep the global newest-first row order and
    // are processed in first-encountered order.
    let mut group_order: Vec<(String, String)> = Vec::new();
    let mut groups: HashMap<(String, String), Vec<releases_repo::Model>> = HashMap::new();
    for model in rows {
        let key = (model.source_kind.clone(), model.source_name.clone());
        if !groups.contains_key(&key) {
            group_order.push(key.clone());
        }
        groups.entry(key).or_default().push(model);
    }

    let mut summary = ReenrichSummary {
        considered,
        ..Default::default()
    };

    for key in group_order {
        let group = groups.remove(&key).unwrap_or_default();
        let (kind, name) = key;
        let Some(enricher) = enrichers.get(&kind) else {
            tracing::warn!(
                source = %name,
                kind = %kind,
                rows = group.len(),
                "no detail-fetching enricher registered for this kind; skipping its rows"
            );
            summary.skipped_no_enricher += group.len();
            continue;
        };

        // Borrow the origin's `poll_runs` lane for the audit row + progress
        // checkpoint, mirroring backfill. Non-fatal if the insert fails; the
        // loop still runs and emits SSE frames only.
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
            events.clone(),
            JobKind::Source,
            &name,
        );
        progress.set_total(group.len() as u64).await;
        progress.set_phase("re-enriching").await;

        let mut reenriched = 0usize;
        let mut errors = 0usize;
        let mut enrich_total_ms: u128 = 0;
        let group_len = group.len();
        for (idx, model) in group.into_iter().enumerate() {
            let mut release = releases_repo::model_to_discovered(&model);
            let enrich_started = Instant::now();
            let enrich_result = enricher.enrich(&mut release).await;
            enrich_total_ms += enrich_started.elapsed().as_millis();
            if let Err(e) = enrich_result {
                tracing::warn!(
                    error = ?e,
                    source = %name,
                    external_id = %release.external_id,
                    "re-enrich detail fetch failed; leaving row as-is"
                );
                errors += 1;
                progress.tick_to((idx + 1) as u64).await;
                continue;
            }
            // persist_discovered upserts (resolution columns are excluded
            // from the on-conflict update set, so the link/status survive)
            // and re-attaches detected formats. No resolver call: re-enrich
            // must not change a release's match.
            match releases_repo::persist_discovered(&db, &release, Utc::now().timestamp()).await {
                Ok(_) => reenriched += 1,
                Err(e) => {
                    tracing::error!(
                        error = ?e,
                        source = %name,
                        external_id = %release.external_id,
                        "re-enrich persist failed"
                    );
                    errors += 1;
                }
            }
            progress.tick_to((idx + 1) as u64).await;
        }
        progress.flush().await;

        let enrich_duration_ms = enrich_total_ms.min(i64::MAX as u128) as i64;
        tracing::info!(
            source = %name,
            considered = group_len,
            reenriched,
            errors,
            enrich_duration_ms,
            "re-enrich group complete"
        );
        summary.reenriched += reenriched;
        summary.errors += errors;

        // Finalize the borrowed poll_runs row. `new`/`resolved` are zero: a
        // re-enrich discovers no rows and decides no matches. `fetched`
        // records the rows walked so the in-flight pill clears with an
        // honest count.
        if let Some(id) = metrics_id {
            let finished_at = Utc::now().timestamp();
            if let Err(e) = run_metrics_repo::finalize_poll_run(
                &db,
                id,
                finished_at,
                run_metrics_repo::status::SUCCESS,
                PollRunCounts {
                    fetched: Some(group_len as i32),
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
    }

    tracing::info!(
        considered = summary.considered,
        reenriched = summary.reenriched,
        errors = summary.errors,
        skipped_no_enricher = summary.skipped_no_enricher,
        "re-enrich complete"
    );

    Ok(summary)
}
