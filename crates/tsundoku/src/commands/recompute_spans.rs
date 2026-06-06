//! `tsundoku recompute-spans`.
//!
//! One-shot, network-free recompute of every release's volume/chapter span
//! and every series' `highest_volume` / `highest_chapter` mark from the
//! stored file lists (falling back to release titles). Run it after changing
//! the span-parsing logic, or to backfill a catalog whose releases predate
//! span detection.
//!
//! Authoritative, not incremental: a series' marks are *replaced* with the
//! MAX across its currently-linked releases, so this also corrects values an
//! earlier parse over-counted. Idempotent. Prefer running it while `serve`
//! is stopped — both processes writing the same SQLite file contend for the
//! single write lock.

use std::path::PathBuf;

use anyhow::{Context, Result};
use td_db::repos::releases_repo;

pub async fn run(config_path: PathBuf) -> Result<()> {
    let cfg = td_config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    super::init_tracing(&cfg);

    let db = td_db::connect(&cfg).await?;
    td_db::run_migrations(&db).await?;

    let now = chrono::Utc::now().timestamp();
    let summary = releases_repo::recompute_all_spans(&db, now).await?;

    println!("\nrecompute-spans summary:");
    println!("  releases rewritten  {}", summary.releases_rewritten);
    println!("  series updated       {}", summary.series_updated);

    Ok(())
}
