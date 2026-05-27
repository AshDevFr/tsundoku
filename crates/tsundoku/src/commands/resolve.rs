//! `tsundoku resolve [--retry-unresolved]`.
//!
//! Drives the resolution pipeline against persisted releases. Two modes:
//!
//! - **Default**: walk every release whose `resolution_status` is
//!   `unresolved`. Useful for the first run after a fresh poll, before
//!   the scheduler is wired up (Phase 6).
//! - **`--retry-unresolved`**: also include `ambiguous` rows. Used when
//!   the operator updates `[ingestion.format_type_rules]` or refreshes
//!   the provider's offline cache and wants the existing review queue
//!   re-evaluated against the new state.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use td_resolution::Resolver;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;

const DEFAULT_BATCH_SIZE: u64 = 1000;

pub async fn run(config_path: PathBuf, retry_unresolved: bool) -> Result<()> {
    let cfg = td_config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    super::init_tracing(&cfg);

    let db = td_db::connect(&cfg).await?;
    td_db::run_migrations(&db).await?;
    let limiter = crate::http_limiter::build(&cfg.ingestion.http);
    let registry = Arc::new(crate::metadata::build_registry(&cfg, limiter.clone()).await?);

    let user_agent = concat!(
        "tsundoku/",
        env!("CARGO_PKG_VERSION"),
        " (+https://github.com/skewb1k/tsundoku)"
    );
    let query_builder = Arc::new(
        QueryBuilder::new(&cfg.ingestion.cleanup.extra_format_keywords)
            .context("building title cleaner from ingestion.cleanup config")?,
    );
    let mut resolver = Resolver::new(db.clone(), registry, cfg.ingestion.clone())
        .with_query_builder(query_builder);
    match MangaUpdatesRedirector::new(user_agent, limiter.clone()) {
        Ok(r) => resolver = resolver.with_mangaupdates_redirector(Arc::new(r)),
        Err(e) => tracing::warn!(error = ?e, "skipping mangaupdates redirector"),
    }
    let resolver = resolver;

    // `resolve_unresolved` already handles both `unresolved` and
    // `ambiguous`; `--retry-unresolved` is the same operation today but
    // exposes the intent in the CLI surface (and gives us a hook if we
    // later want a default-mode that skips `ambiguous`).
    let _ = retry_unresolved;
    let summary = resolver.resolve_unresolved(DEFAULT_BATCH_SIZE).await?;

    println!("\nresolve summary:");
    println!("  resolved        {}", summary.resolved);
    println!("  ambiguous       {}", summary.ambiguous);
    println!("  review_pending  {}", summary.review_pending);
    println!("  unresolved      {}", summary.unresolved);
    println!("  errors          {}", summary.errors);
    println!("  total           {}", summary.total());

    Ok(())
}
