use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;

use crate::api;

pub async fn run(config_path: PathBuf) -> anyhow::Result<()> {
    let cfg = td_config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    super::init_tracing(&cfg);

    let db = td_db::connect(&cfg).await?;
    td_db::run_migrations(&db).await?;

    let state = Arc::new(api::AppState { db });
    let app = api::router(state, &cfg);

    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "listening");
    axum::serve(listener, app).await?;
    Ok(())
}
