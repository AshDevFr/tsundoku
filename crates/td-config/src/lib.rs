//! Layered configuration: struct defaults -> config file -> environment.
//!
//! The config file may be TOML or YAML; the provider is chosen from the file
//! extension. Environment variables use the `TSUNDOKU_` prefix with
//! `__` as the nesting separator, e.g. `TSUNDOKU_SERVER__PORT=9000`.

use std::path::{Path, PathBuf};

use anyhow::Context;
use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml, Yaml};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub logging: LoggingConfig,
    pub api: ApiConfig,
    pub metadata: MetadataConfig,
    pub providers: ProvidersConfig,
    /// Configured discovery sources. Each entry is one instance (kind +
    /// name + cron + per-kind options). Order is not significant; the
    /// scheduler keys on `name`.
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

/// On-disk storage layout. `data_dir` is the single root; the other paths
/// default to subdirectories of it but may be explicitly overridden.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub data_dir: PathBuf,
    pub database_path: Option<PathBuf>,
    pub provider_cache_dir: Option<PathBuf>,
    pub cover_cache_dir: Option<PathBuf>,
    pub tmp_dir: Option<PathBuf>,
}

impl StorageConfig {
    /// Resolve all paths against `data_dir`, returning a fully-rooted
    /// [`StoragePaths`] that callers can use directly. Does not touch the
    /// filesystem; call [`StoragePaths::ensure`] for that.
    pub fn paths(&self) -> StoragePaths {
        StoragePaths {
            database_path: self
                .database_path
                .clone()
                .unwrap_or_else(|| self.data_dir.join("db").join("tsundoku.db")),
            provider_cache_dir: self
                .provider_cache_dir
                .clone()
                .unwrap_or_else(|| self.data_dir.join("cache").join("providers")),
            cover_cache_dir: self
                .cover_cache_dir
                .clone()
                .unwrap_or_else(|| self.data_dir.join("cache").join("covers")),
            tmp_dir: self
                .tmp_dir
                .clone()
                .unwrap_or_else(|| self.data_dir.join("tmp")),
            data_dir: self.data_dir.clone(),
        }
    }
}

/// Fully-resolved on-disk paths. Construct via [`StorageConfig::paths`].
#[derive(Debug, Clone)]
pub struct StoragePaths {
    pub data_dir: PathBuf,
    pub database_path: PathBuf,
    pub provider_cache_dir: PathBuf,
    pub cover_cache_dir: PathBuf,
    pub tmp_dir: PathBuf,
}

impl StoragePaths {
    /// SQLite connection string targeting [`Self::database_path`]. WAL and
    /// the per-connection pragmas are applied separately by `td-db::conn`.
    pub fn database_url(&self) -> String {
        format!("sqlite://{}?mode=rwc", self.database_path.display())
    }

    /// Provider-owned subdirectory under `provider_cache_dir/<provider>/`.
    /// Used by metadata providers that maintain a local cache on disk.
    pub fn provider_cache_dir_for(&self, provider: &str) -> PathBuf {
        self.provider_cache_dir.join(provider)
    }

    /// Create every directory in this layout if it does not already exist.
    /// The database's parent directory is included; the database file itself
    /// is created by SQLite on first connect.
    pub fn ensure(&self) -> anyhow::Result<()> {
        for dir in [
            &self.data_dir,
            &self.provider_cache_dir,
            &self.cover_cache_dir,
            &self.tmp_dir,
        ] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        if let Some(parent) = self.database_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        Ok(())
    }
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

/// Metadata-layer settings independent of any specific provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetadataConfig {
    /// The provider that runs the auto-detect resolution path. Must match
    /// a provider id registered in [`ProvidersConfig`].
    pub active_provider: String,
}

/// Per-provider config blocks. Adding a new provider = adding a field here +
/// wiring its construction in the registry builder. Each provider type stays
/// fully typed (no opaque JSON), which keeps schema mistakes loud at boot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    pub mangabaka: MangabakaProviderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MangabakaProviderConfig {
    /// Toggle without removing the block. Disabled providers are not
    /// registered, so the active-provider validation will fail if you
    /// disable the one currently selected.
    pub enabled: bool,
    /// API key sent as the `x-api-key` header.
    pub api_key: Option<String>,
    /// API base URL. Override only when proxying.
    pub api_base_url: String,
    /// Published offline dump URL (tarball of the MangaBaka dataset).
    /// `None` disables the offline cache and forces API-only operation.
    pub offline_dump_url: Option<String>,
    /// Cron expression for the scheduled cache refresh job. `None` disables
    /// the scheduled refresh; manual `tsundoku refresh-metadata` still works.
    pub offline_refresh_cron: Option<String>,
    /// Fall back to the live API when the offline cache misses an ID.
    pub api_fallback: bool,
    /// TTL for the negative cache (known-misses) in days.
    pub negative_cache_ttl_days: u32,
    /// HTTP timeout per request, in seconds.
    pub timeout_seconds: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8080,
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("./data"),
            database_path: None,
            provider_cache_dir: None,
            cover_cache_dir: None,
            tmp_dir: None,
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

impl Default for MetadataConfig {
    fn default() -> Self {
        Self {
            active_provider: "mangabaka".into(),
        }
    }
}

impl Default for MangabakaProviderConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_key: None,
            api_base_url: "https://api.mangabaka.dev".into(),
            offline_dump_url: None,
            offline_refresh_cron: None,
            // api_fallback defaults to false so a freshly-installed binary
            // boots cleanly without an api_key. Operators opt in by setting
            // api_key + api_fallback = true.
            api_fallback: false,
            negative_cache_ttl_days: 7,
            timeout_seconds: 60,
        }
    }
}

/// One configured discovery source. The `kind` field is a discriminator the
/// source registry uses to pick a constructor; per-kind options live in the
/// optional nested blocks. v1 only ships `nyaa`; adding a new kind = adding
/// an optional nested-options field below + a registry constructor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    /// Source kind discriminator (e.g. `"nyaa"`).
    pub kind: String,
    /// Instance name, unique across the registry. Persisted as
    /// `releases.source_name`.
    pub name: String,
    /// Cron expression for the scheduler. Only used by the scheduled
    /// poller; the one-shot `tsundoku poll --source <name>` ignores it.
    #[serde(default)]
    pub cron: Option<String>,
    /// Disabled sources are skipped at registry build time.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Per-kind nested options. The relevant variant must match `kind`.
    #[serde(default)]
    pub nyaa: Option<NyaaSourceOptions>,
}

fn default_true() -> bool {
    true
}

/// Nyaa-specific options, populated from `[sources.nyaa]` under the matching
/// `[[sources]]` entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NyaaSourceOptions {
    pub feed_url: String,
    /// HTTP timeout (seconds) for the feed and detail fetches.
    pub timeout_seconds: u32,
    /// Fetch each post's detail HTML after the RSS pass to enrich the file
    /// list and external links. Off by default — keeps the poll cheap.
    pub fetch_details: bool,
    /// Override for the site base URL. Useful when the feed is proxied.
    pub site_base_url: String,
}

impl Default for NyaaSourceOptions {
    fn default() -> Self {
        Self {
            feed_url: String::new(),
            timeout_seconds: 30,
            fetch_details: false,
            site_base_url: "https://nyaa.si".into(),
        }
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
        assert_eq!(cfg.metadata.active_provider, "mangabaka");
        assert_eq!(cfg.storage.data_dir, PathBuf::from("./data"));
        assert!(cfg.providers.mangabaka.enabled);
        assert_eq!(
            cfg.providers.mangabaka.api_base_url,
            "https://api.mangabaka.dev"
        );
    }

    #[test]
    fn storage_paths_derive_subdirs_from_data_dir() {
        let storage = StorageConfig {
            data_dir: PathBuf::from("/var/tsundoku"),
            ..Default::default()
        };
        let p = storage.paths();
        assert_eq!(
            p.database_path,
            PathBuf::from("/var/tsundoku/db/tsundoku.db")
        );
        assert_eq!(
            p.provider_cache_dir,
            PathBuf::from("/var/tsundoku/cache/providers")
        );
        assert_eq!(
            p.cover_cache_dir,
            PathBuf::from("/var/tsundoku/cache/covers")
        );
        assert_eq!(p.tmp_dir, PathBuf::from("/var/tsundoku/tmp"));
        assert_eq!(
            p.provider_cache_dir_for("mangabaka"),
            PathBuf::from("/var/tsundoku/cache/providers/mangabaka")
        );
        assert_eq!(
            p.database_url(),
            "sqlite:///var/tsundoku/db/tsundoku.db?mode=rwc"
        );
    }

    #[test]
    fn sources_array_parses_with_nested_options() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tsundoku.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[storage]
data_dir = "./data"

[metadata]
active_provider = "mangabaka"

[[sources]]
kind = "nyaa"
name = "trusted"
cron = "*/30 * * * *"
enabled = true
  [sources.nyaa]
  feed_url = "https://nyaa.si/?page=rss&f=2"

[[sources]]
kind = "nyaa"
name = "english-manga"
cron = "0 */2 * * *"
  [sources.nyaa]
  feed_url = "https://nyaa.si/?page=rss&c=3_1"
            "#
        )
        .unwrap();

        let cfg = load(&path).unwrap();
        assert_eq!(cfg.sources.len(), 2);
        assert_eq!(cfg.sources[0].name, "trusted");
        assert_eq!(cfg.sources[0].kind, "nyaa");
        assert!(cfg.sources[0].enabled);
        assert_eq!(cfg.sources[0].cron.as_deref(), Some("*/30 * * * *"));
        let opts = cfg.sources[0]
            .nyaa
            .as_ref()
            .expect("nyaa nested options should be present");
        assert_eq!(opts.feed_url, "https://nyaa.si/?page=rss&f=2");
        // Defaults from NyaaSourceOptions::default still apply to omitted fields.
        assert_eq!(opts.timeout_seconds, 30);
        assert!(!opts.fetch_details);
        // `enabled` defaults to true when omitted.
        assert!(cfg.sources[1].enabled);
    }

    #[test]
    fn storage_paths_honor_explicit_overrides() {
        let storage = StorageConfig {
            data_dir: PathBuf::from("/var/tsundoku"),
            database_path: Some(PathBuf::from("/srv/sqlite/td.db")),
            ..Default::default()
        };
        let p = storage.paths();
        assert_eq!(p.database_path, PathBuf::from("/srv/sqlite/td.db"));
        // Untouched fields still derive from data_dir.
        assert_eq!(
            p.provider_cache_dir,
            PathBuf::from("/var/tsundoku/cache/providers")
        );
    }
}
