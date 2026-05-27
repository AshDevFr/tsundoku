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
//! API handler call [`run`]; keeping the loop here (not in the binary)
//! means the manual API trigger shares the same per-source mutex the cron
//! poll holds, so a backfill can't race a scheduled tick. Idempotent on
//! `(source_kind, external_id)`; never touches `source_state`, so it does
//! not move the cron's ETag / last-poll markers.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::Utc;
use sea_orm::DatabaseConnection;
use td_config::IngestionConfig;
use td_db::repos::releases_repo::{self, id_for};
use td_metadata::MetadataRegistry;
use td_resolution::Resolver;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;
use td_source::DiscoverySource;

use crate::JobLocks;

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

/// Result of a [`run`] invocation.
pub enum BackfillOutcome {
    /// Backfill ran to completion (or stopped early on an empty/failed page).
    Ran(BackfillSummary),
    /// Another poll or backfill for this source held the per-source mutex;
    /// nothing ran. Same semantics as a skipped poll tick.
    Skipped,
}

/// Walk up to `pages` listing pages for `source`, persisting and resolving
/// every new release. Acquires the per-source mutex first (returning
/// [`BackfillOutcome::Skipped`] if a poll/backfill is already in flight),
/// so the manual API trigger and the cron poll can't race.
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
    locks: Arc<JobLocks>,
    query_builder: Arc<QueryBuilder>,
    mangaupdates_redirector: Option<Arc<MangaUpdatesRedirector>>,
    pages: u32,
    trigger: &str,
) -> Result<BackfillOutcome> {
    let pages = pages.max(1);
    let name = source.name().to_string();
    let kind = source.kind().to_string();

    // Reject non-backfillable sources before taking the lock, so the error
    // is about capability rather than contention.
    if source.as_backfillable().is_none() {
        return Err(anyhow!(
            "source {name:?} (kind={kind}) does not support historical backfill"
        ));
    }

    let lock = locks.source_lock(&name);
    let Ok(_guard) = lock.try_lock() else {
        tracing::debug!(source = %name, "poll/backfill already running; skipping backfill");
        return Ok(BackfillOutcome::Skipped);
    };

    let mut resolver =
        Resolver::new(db.clone(), metadata, ingestion).with_query_builder(query_builder);
    if let Some(r) = mangaupdates_redirector {
        resolver = resolver.with_mangaupdates_redirector(r);
    }

    let backfillable = source
        .as_backfillable()
        .expect("as_backfillable checked Some above");

    tracing::info!(source = %name, kind = %kind, pages, trigger = %trigger, "backfill starting");

    let mut totals = BackfillSummary::default();
    for page in 1..=pages {
        let releases = match backfillable.backfill_page(page).await {
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
            if let Err(e) = source.enrich(&mut release).await {
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
            if let Err(e) = resolver.resolve_one(&persisted_id).await {
                tracing::warn!(error = ?e, release_id = %persisted_id, "resolver failed; release left unresolved");
                page_errors += 1;
            }
            page_new += 1;
        }
        tracing::info!(
            source = %name,
            page,
            total = page_total,
            new = page_new,
            skipped = page_skipped,
            errors = page_errors,
            "backfill page complete"
        );
        totals.pages_walked += 1;
        totals.total += page_total;
        totals.new += page_new;
        totals.already_known += page_skipped;
        totals.errors += page_errors;
    }

    Ok(BackfillOutcome::Ran(totals))
}
