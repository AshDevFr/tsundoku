use std::path::PathBuf;

use anyhow::Context;

pub async fn run(config_path: PathBuf) -> anyhow::Result<()> {
    let cfg = td_config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    super::init_tracing(&cfg);

    let db = td_db::connect(&cfg).await?;
    td_db::run_migrations(&db).await?;
    tracing::info!("migrations complete");
    Ok(())
}
