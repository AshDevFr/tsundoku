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

const DEFAULT_BATCH_SIZE: u64 = 1000;

pub async fn run(config_path: PathBuf, retry_unresolved: bool) -> Result<()> {
    let cfg = td_config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    super::init_tracing(&cfg);

    let db = td_db::connect(&cfg).await?;
    td_db::run_migrations(&db).await?;
    let registry = Arc::new(crate::metadata::build_registry(&cfg).await?);

    let resolver = Resolver::new(db.clone(), registry, cfg.ingestion.clone());

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
