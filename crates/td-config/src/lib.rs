//! Layered configuration: struct defaults -> config file -> environment.
//!
//! The config file may be TOML or YAML; the provider is chosen from the file
//! extension. Environment variables use the `TSUNDOKU_` prefix with
//! `__` as the nesting separator, e.g. `TSUNDOKU_SERVER__PORT=9000`.

use std::path::Path;

use anyhow::Context;
use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml, Yaml};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub logging: LoggingConfig,
    pub api: ApiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
    /// Emit JSON log lines instead of the human-readable format.
    pub json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiConfig {
    /// Mount the Scalar API docs UI at `/docs`.
    pub docs: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8080,
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: "sqlite://data/tsundoku.db?mode=rwc".into(),
            max_connections: 16,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            json: false,
        }
    }
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self { docs: true }
    }
}

/// Load configuration, applying defaults, then the file (if present), then env.
pub fn load(path: &Path) -> anyhow::Result<AppConfig> {
    let mut fig = Figment::from(Serialized::defaults(AppConfig::default()));

    if path.exists() {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        fig = match ext {
            "yaml" | "yml" => fig.merge(Yaml::file(path)),
            _ => fig.merge(Toml::file(path)),
        };
    }

    fig.merge(Env::prefixed("TSUNDOKU_").split("__"))
        .extract()
        .context("parsing configuration")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn defaults_apply_when_no_file() {
        let cfg = load(&PathBuf::from("does-not-exist.toml")).unwrap();
        assert_eq!(cfg.server.port, 8080);
        assert_eq!(cfg.logging.level, "info");
    }
}
