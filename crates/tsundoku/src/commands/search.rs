//! `tsundoku search --series <id> [--entry <name>]`.
//!
//! One-shot per-series release search. Builds the same search registry,
//! metadata registry, and resolver the `serve` process uses, then drives
//! [`td_scheduler::jobs::search_series::run`] — the identical loop the
//! series-page button runs. Idempotent on re-runs: already-known releases
//! are skipped on `(source_kind, external_id)`.
//!
//! Safe-concurrency note: like `backfill`, this is a *separate process*
//! from `serve`, so the per-entry mutex used by the API trigger does not
//! coordinate with it. Redundant work at worst, not corruption.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use td_db::repos::search_runs_repo;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;
use td_scheduler::jobs::search_series;

pub async fn run(config_path: PathBuf, series_id: i32, entry_name: Option<String>) -> Result<()> {
    let cfg = td_config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    super::init_tracing(&cfg);

    let db = td_db::connect(&cfg).await?;
    td_db::run_migrations(&db).await?;
    let limiter = crate::http_limiter::build(&cfg.ingestion.http);
    let registry = crate::search_registry::build_search_registry(&cfg, limiter.clone())?;

    let entry = match &entry_name {
        Some(name) => registry
            .get(name)
            .ok_or_else(|| anyhow!("search entry {name:?} is not configured (or is disabled)"))?,
        None => registry.default_entry().ok_or_else(|| {
            anyhow!("no [[search]] entries are configured; add one to the config first")
        })?,
    };
    let source = entry.source.clone();
    let max_pages = entry.max_pages;

    let metadata = Arc::new(crate::metadata::build_registry(&cfg, limiter.clone()).await?);
    let user_agent = concat!(
        "tsundoku/",
        env!("CARGO_PKG_VERSION"),
        " (+https://github.com/AshDevFr/tsundoku)"
    );
    let query_builder = Arc::new(
        QueryBuilder::new(&cfg.ingestion.cleanup.extra_format_keywords)
            .context("building title cleaner from ingestion.cleanup config")?,
    );
    let mu_redirector = match MangaUpdatesRedirector::new(user_agent, limiter.clone()) {
        Ok(r) => Some(Arc::new(r)),
        Err(e) => {
            tracing::warn!(error = ?e, "skipping mangaupdates redirector");
            None
        }
    };

    let name = source.name().to_string();
    let kind = source.kind().to_string();
    let totals = search_series::run(
        source,
        max_pages,
        db,
        metadata,
        cfg.ingestion.clone(),
        query_builder,
        mu_redirector,
        series_id,
        search_runs_repo::TRIGGER_CLI,
    )
    .await?;

    println!("\nsearch summary:");
    println!("  entry            {name} ({kind})");
    println!("  series_id        {series_id}");
    println!("  queries          {}", totals.queries_attempted);
    if totals.queries_dropped > 0 {
        println!("  queries_dropped  {}", totals.queries_dropped);
    }
    println!("  pages_fetched    {}", totals.pages_fetched);
    println!("  hits_seen        {}", totals.releases_seen);
    println!("  new              {}", totals.releases_new);
    println!("  already_known    {}", totals.already_known);
    println!("  errors           {}", totals.errors);

    Ok(())
}
