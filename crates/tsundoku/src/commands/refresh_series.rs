//! `tsundoku refresh-series [--batch-size N] [--min-age-days N]`.
//!
//! One-shot bulk refresh of stale series rows against the active
//! metadata provider. Shares the same tick code as the scheduler cron
//! and the `POST /api/v1/series/refresh-all` endpoint, so behaviour is
//! identical regardless of which surface fires it. The same per-provider
//! mutex applies: if a scheduler tick or another manual trigger is in
//! flight, this command records a `skipped` row and exits.
//!
//! Flag defaults pull from `metadata.series_refresh.{batch_size,
//! min_age_days}`; pass `--batch-size 0` to make the tick a no-op
//! without touching config.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use td_db::repos::run_metrics_repo;
use td_metadata::MetadataProvider;
use td_scheduler::{JobLocks, jobs::refresh_series_metadata};

pub async fn run(
    config_path: PathBuf,
    batch_size: Option<u32>,
    min_age_days: Option<u32>,
) -> anyhow::Result<()> {
    let cfg = td_config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    super::init_tracing(&cfg);

    let db = td_db::connect(&cfg).await?;
    td_db::run_migrations(&db).await?;

    let limiter = crate::http_limiter::build(&cfg.ingestion.http);
    let registry = crate::metadata::build_registry(&cfg, limiter).await?;

    let active_id = cfg.metadata.active_provider.clone();
    let provider: Arc<dyn MetadataProvider> = registry
        .get(&active_id)
        .ok_or_else(|| anyhow::anyhow!("active provider {active_id:?} is not registered"))?
        .clone();

    let batch_size = batch_size.unwrap_or(cfg.metadata.series_refresh.batch_size);
    let min_age_days = min_age_days.unwrap_or(cfg.metadata.series_refresh.min_age_days);
    let min_age_seconds = (min_age_days as i64).saturating_mul(86_400);

    tracing::info!(
        provider = %active_id,
        batch_size,
        min_age_days,
        "refresh-series: kicking off one tick"
    );

    let locks = Arc::new(JobLocks::default());
    // CLI has no SSE consumer; a detached sender keeps the tick signature
    // consistent with the API/cron paths.
    let (events, _) = tokio::sync::broadcast::channel(16);
    refresh_series_metadata::run_tick(
        provider,
        db.clone(),
        locks,
        batch_size,
        min_age_seconds,
        events,
        run_metrics_repo::trigger::MANUAL,
    )
    .await;

    Ok(())
}
