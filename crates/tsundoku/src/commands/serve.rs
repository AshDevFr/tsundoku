use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use td_api::AppState;
use td_scheduler::{JobLocks, Scheduler, SchedulerContext};

pub async fn run(config_path: PathBuf) -> anyhow::Result<()> {
    let cfg = td_config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    super::init_tracing(&cfg);

    let db = td_db::connect(&cfg).await?;
    td_db::run_migrations(&db).await?;

    let sources = Arc::new(crate::source_registry::build_registry(&cfg)?);
    let metadata = Arc::new(crate::metadata::build_registry(&cfg).await?);
    let locks = Arc::new(JobLocks::default());

    let ctx = SchedulerContext {
        db: db.clone(),
        sources: sources.clone(),
        metadata: metadata.clone(),
        ingestion: cfg.ingestion.clone(),
        locks: locks.clone(),
    };
    let scheduler = Scheduler::build(&cfg, ctx).await?;
    scheduler.start().await?;

    let state = AppState {
        db,
        sources,
        metadata,
        ingestion: cfg.ingestion.clone(),
        auth: Arc::new(cfg.auth.clone()),
        locks,
        sources_config: Arc::new(cfg.sources.clone()),
        providers_config: Arc::new(cfg.providers.clone()),
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
