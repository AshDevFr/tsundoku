use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use sea_orm::DatabaseConnection;
use td_api::AppState;
use td_db::repos::run_metrics_repo::trigger;
use td_metadata::MetadataRegistry;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;
use td_scheduler::{JobLocks, Scheduler, SchedulerContext, jobs::refresh_provider_cache};

pub async fn run(config_path: PathBuf) -> anyhow::Result<()> {
    let cfg = td_config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    super::init_tracing(&cfg);

    let db = td_db::connect(&cfg).await?;
    td_db::run_migrations(&db).await?;

    // Build the process-wide outbound-HTTP limiter first; every component
    // that makes external requests routes through it so per-host
    // serialization and min-gap are enforced uniformly.
    let limiter = crate::http_limiter::build(&cfg.ingestion.http);

    let sources = Arc::new(crate::source_registry::build_registry(
        &cfg,
        limiter.clone(),
    )?);
    let metadata = Arc::new(crate::metadata::build_registry(&cfg, limiter.clone()).await?);
    let locks = Arc::new(JobLocks::default());
    // One shared MangaUpdates redirector for the whole process: scheduler
    // ticks and the API retry handler both go through the same throttle.
    let user_agent = concat!(
        "tsundoku/",
        env!("CARGO_PKG_VERSION"),
        " (+https://github.com/skewb1k/tsundoku)"
    );
    let mu_redirector = match MangaUpdatesRedirector::new(user_agent, limiter.clone()) {
        Ok(r) => Some(Arc::new(r)),
        Err(e) => {
            tracing::warn!(error = ?e, "failed to build mangaupdates redirector; legacy MU URLs will be dropped");
            None
        }
    };
    // Build the title cleaner once: built-in keyword list + operator
    // extras from `[ingestion.cleanup]`. Invalid extras (regex
    // metacharacters, empty strings) make the binary refuse to start.
    let query_builder = Arc::new(
        QueryBuilder::new(&cfg.ingestion.cleanup.extra_format_keywords)
            .context("building title cleaner from ingestion.cleanup config")?,
    );

    let ctx = SchedulerContext {
        db: db.clone(),
        sources: sources.clone(),
        metadata: metadata.clone(),
        ingestion: cfg.ingestion.clone(),
        locks: locks.clone(),
        query_builder: query_builder.clone(),
        mangaupdates_redirector: mu_redirector.clone(),
    };
    let scheduler = Scheduler::build(&cfg, ctx).await?;
    scheduler.start().await?;

    // If any provider has an offline cache configured but no dump on disk
    // yet, kick off an immediate refresh in the background so the operator
    // does not have to wait for the next cron tick. The job goes through the
    // same locked code path as the scheduler tick, so a concurrently firing
    // cron just `try_lock`s and skips. Errors are logged inside `run_tick`.
    spawn_startup_refreshes(metadata.clone(), db.clone(), locks.clone());

    let (job_events, _) = tokio::sync::broadcast::channel(td_api::JOB_EVENT_BUFFER);
    let state = AppState {
        db,
        sources,
        metadata,
        ingestion: cfg.ingestion.clone(),
        auth: Arc::new(cfg.auth.clone()),
        locks,
        sources_config: Arc::new(cfg.sources.clone()),
        providers_config: Arc::new(cfg.providers.clone()),
        metadata_config: Arc::new(cfg.metadata.clone()),
        query_builder,
        mangaupdates_redirector: mu_redirector,
        job_events,
    };
    let app = td_api::router(state, &cfg);

    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "listening");
    axum::serve(listener, app).await?;

    // axum::serve returns when the listener is closed (Ctrl-C on most
    // platforms). Drop the scheduler explicitly so any in-flight tick is
    // cancelled before we exit.
    drop(scheduler);
    Ok(())
}

/// Inspect every registered provider and, for those that don't yet have a
/// loaded offline cache, spawn an off-the-runtime task to perform a one-shot
/// refresh. Providers that don't support an offline cache return
/// `RefreshStatus::NotSupported`, which the tick treats as a no-op.
fn spawn_startup_refreshes(
    metadata: Arc<MetadataRegistry>,
    db: DatabaseConnection,
    locks: Arc<JobLocks>,
) {
    for (id, provider) in metadata.iter() {
        let id = id.to_string();
        let provider = provider.clone();
        let db = db.clone();
        let locks = locks.clone();
        tokio::spawn(async move {
            if provider.offline_cache_loaded().await {
                tracing::debug!(provider = %id, "offline cache already loaded; skipping startup refresh");
                return;
            }
            tracing::info!(provider = %id, "no offline cache on disk; refreshing at startup");
            refresh_provider_cache::run_tick(provider, db, locks, trigger::STARTUP).await;
        });
    }
}
