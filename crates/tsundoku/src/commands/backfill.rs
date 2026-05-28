//! `tsundoku backfill <source> --pages N`.
//!
//! One-shot historical catch-up. Builds the same source registry,
//! metadata registry, and resolver the `serve` process uses, then drives
//! [`td_scheduler::jobs::backfill_source::run`] — the identical loop the
//! `POST /sources/{name}/backfill` endpoint runs. Idempotent on re-runs;
//! never touches `source_state`, so it does not move the cron's ETag /
//! last-poll markers.
//!
//! Safe-concurrency note: this is a *separate process* from `serve`. The
//! per-source mutex it acquires is process-local, so it does not
//! coordinate with a running `serve`'s cron polls — two processes could
//! resolve the same source at once (redundant work, not corruption, as
//! long as the DB lives on a real filesystem rather than a macOS bind
//! mount). For coordinated catch-up while `serve` is up, prefer the
//! `POST /sources/{name}/backfill` endpoint, which runs in-process under
//! the shared lock.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;
use td_scheduler::JobLocks;
use td_scheduler::jobs::backfill_source::{self, BackfillOutcome};

pub async fn run(config_path: PathBuf, source_name: String, pages: u32) -> Result<()> {
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
    let mu_redirector = match MangaUpdatesRedirector::new(user_agent, limiter.clone()) {
        Ok(r) => Some(Arc::new(r)),
        Err(e) => {
            tracing::warn!(error = ?e, "skipping mangaupdates redirector");
            None
        }
    };

    // Process-local locks: meaningful only within this CLI invocation. See
    // the module-level note on cross-process coordination.
    let locks = Arc::new(JobLocks::default());

    let kind = source.kind().to_string();
    // CLI has no SSE consumer; detached sender for signature parity with
    // the API path.
    let (events, _) = tokio::sync::broadcast::channel(16);
    let outcome = backfill_source::run(
        source,
        db,
        metadata,
        cfg.ingestion.clone(),
        locks,
        query_builder,
        mu_redirector,
        events,
        pages,
        "cli",
    )
    .await?;

    match outcome {
        BackfillOutcome::Ran(totals) => {
            println!("\nbackfill summary:");
            println!("  source         {source_name} ({kind})");
            println!("  pages_walked   {}", totals.pages_walked);
            println!("  rows_seen      {}", totals.total);
            println!("  new            {}", totals.new);
            println!("  already_known  {}", totals.already_known);
            println!("  errors         {}", totals.errors);
        }
        BackfillOutcome::Skipped => {
            // Only reachable if something else in this same process holds
            // the lock, which the CLI never does — defensive.
            println!("backfill skipped: another run for {source_name:?} is already in flight");
        }
    }

    Ok(())
}
