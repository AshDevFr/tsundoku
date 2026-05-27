pub mod backfill;
pub mod migrate;
pub mod openapi;
pub mod poll;
pub mod refresh_metadata;
pub mod resolve;
pub mod serve;

use td_config::AppConfig;
use tracing_subscriber::EnvFilter;

/// Initialize tracing once per process. `RUST_LOG` overrides the config level.
/// sqlx query logging is pinned to `warn` unless explicitly raised.
pub fn init_tracing(cfg: &AppConfig) {
    let filter =
        std::env::var("RUST_LOG").unwrap_or_else(|_| format!("{},sqlx=warn", cfg.logging.level));
    let builder = tracing_subscriber::fmt().with_env_filter(EnvFilter::new(filter));
    if cfg.logging.json {
        builder.json().init();
    } else {
        builder.init();
    }
}
