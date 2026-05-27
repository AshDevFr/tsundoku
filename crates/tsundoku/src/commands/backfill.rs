//! `tsundoku backfill <source> --pages N`.
//!
//! One-shot historical catch-up. Walks the source's HTML listing pages
//! (which actually paginate, unlike Nyaa's RSS feed), persists every new
//! release, and runs the resolver on each. Reuses the existing
//! [`DiscoverySource::enrich`] hook so detail-page extraction matches the
//! steady-state poll path.
//!
//! Idempotent: rows whose `(source_kind, external_id)` is already present
//! in the local DB are skipped before the detail fetch, so re-running with
//! the same `--pages` is cheap and safe. Never touches `source_state`, so
//! it does not affect the cron's ETag / last-poll markers.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use td_db::repos::{releases_repo, releases_repo::id_for};
use td_resolution::Resolver;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;

pub async fn run(config_path: PathBuf, source_name: String, pages: u32) -> Result<()> {
    let pages = pages.max(1);
    let cfg = td_config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    super::init_tracing(&cfg);

    let db = td_db::connect(&cfg).await?;
    td_db::run_migrations(&db).await?;
    let limiter = crate::http_limiter::build(&cfg.ingestion.http);
    let registry = crate::source_registry::build_registry(&cfg, limiter.clone())?;

    let source = registry
        .get(&source_name)
        .ok_or_else(|| anyhow!("source {source_name:?} is not configured (or is disabled)"))?
        .clone();
    let backfillable = source.as_backfillable().ok_or_else(|| {
        anyhow!(
            "source {source_name:?} (kind={}) does not support historical backfill",
            source.kind()
        )
    })?;

    let metadata = Arc::new(crate::metadata::build_registry(&cfg, limiter.clone()).await?);
    let user_agent = concat!(
        "tsundoku/",
        env!("CARGO_PKG_VERSION"),
        " (+https://github.com/skewb1k/tsundoku)"
    );
    let query_builder = Arc::new(
        QueryBuilder::new(&cfg.ingestion.cleanup.extra_format_keywords)
            .context("building title cleaner from ingestion.cleanup config")?,
    );
    let mut resolver = Resolver::new(db.clone(), metadata, cfg.ingestion.clone())
        .with_query_builder(query_builder);
    match MangaUpdatesRedirector::new(user_agent, limiter.clone()) {
        Ok(r) => resolver = resolver.with_mangaupdates_redirector(Arc::new(r)),
        Err(e) => tracing::warn!(error = ?e, "skipping mangaupdates redirector"),
    }
    let resolver = resolver;

    let mut totals = BackfillTotals::default();
    let kind = source.kind().to_string();
    tracing::info!(
        source = %source_name,
        kind = %kind,
        pages,
        "backfill starting"
    );

    for page in 1..=pages {
        let releases = match backfillable.backfill_page(page).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = ?e, source = %source_name, page, "backfill page failed; stopping");
                break;
            }
        };
        if releases.is_empty() {
            tracing::info!(source = %source_name, page, "no more pages; stopping");
            break;
        }
        let page_total = releases.len();
        let mut page_new = 0usize;
        let mut page_skipped = 0usize;
        let mut page_errors = 0usize;
        for mut release in releases {
            let id = id_for(&release.source_kind, &release.external_id);
            // Cheap dedup: if the row already exists, skip the detail
            // fetch + resolver entirely. Re-running with the same --pages
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
                    source = %source_name,
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
            source = %source_name,
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
        totals.skipped += page_skipped;
        totals.errors += page_errors;
    }

    println!("\nbackfill summary:");
    println!("  source         {source_name} ({kind})");
    println!("  pages_walked   {}", totals.pages_walked);
    println!("  rows_seen      {}", totals.total);
    println!("  new            {}", totals.new);
    println!("  already_known  {}", totals.skipped);
    println!("  errors         {}", totals.errors);

    Ok(())
}

#[derive(Default)]
struct BackfillTotals {
    pages_walked: u32,
    total: usize,
    new: usize,
    skipped: usize,
    errors: usize,
}
