//! Layered configuration: struct defaults -> config file -> local overlay
//! -> environment.
//!
//! The config file may be TOML or YAML; the provider is chosen from the file
//! extension. A sibling `<stem>.local.<ext>` file (e.g. `tsundoku.docker.toml`
//! -> `tsundoku.docker.local.toml`) is auto-merged on top of the base file
//! when present, so operators can pin secrets and per-host overrides without
//! touching the committed config. Environment variables use the `TSUNDOKU_`
//! prefix with `__` as the nesting separator, e.g. `TSUNDOKU_SERVER__PORT=9000`,
//! and override both files.

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
    pub auth: AuthConfig,
    pub metadata: MetadataConfig,
    pub providers: ProvidersConfig,
    /// Configured discovery sources. Each entry is one instance (kind +
    /// name + cron + per-kind options). Order is not significant; the
    /// scheduler keys on `name`.
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    pub ingestion: IngestionConfig,
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

/// Single-user auth, populated from `[auth]`. Read endpoints are public by
/// default; flip `read_requires_auth` to gate them behind `api_key`. Write
/// endpoints always require the `admin_token` bearer; if it is unset the
/// service refuses to start.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    /// When true, every `/api/v1/*` GET requires `X-API-Key: <api_key>` (or
    /// `Authorization: Bearer <api_key>`). When false, reads are open.
    pub read_requires_auth: bool,
    /// API key for read endpoints. Required when `read_requires_auth` is
    /// true. Otherwise informational.
    pub api_key: Option<String>,
    /// Bearer token for write endpoints. When `None`, every write returns
    /// 503 (clearer than 401 for a misconfigured deploy).
    pub admin_token: Option<String>,
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
    /// list and external links. On by default: the extra HTTP cost is
    /// dwarfed by the resolver wins from MangaUpdates / AniList URLs the
    /// uploader pasted into the description.
    pub fetch_details: bool,
    /// Override for the site base URL. Useful when the feed is proxied.
    pub site_base_url: String,
}

/// Settings for the resolution pipeline. Controls how raw `releases` rows
/// get resolved to `series` rows: fuzzy-match threshold, review-queue
/// behavior, and format/type validation rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IngestionConfig {
    /// Dice-coefficient threshold above which a fuzzy-title match counts
    /// as a definitive resolution. Below this but plausible, the candidate
    /// lands in `review_candidates`. Range: 0.0..=1.0.
    pub resolution_threshold: f32,
    /// Minimum score required for a candidate to make it into the review
    /// queue. Below this we treat the release as truly unresolved.
    pub review_threshold: f32,
    /// Maximum number of `search()` hits to consider per release in the
    /// fuzzy-title step. Provider may cap further server-side.
    pub fuzzy_search_limit: u32,
    /// When true, releases that fail the fuzzy threshold but produce
    /// plausible candidates land in the review queue. When false, they
    /// stay `unresolved` with no `review_candidates` rows.
    pub queue_low_confidence: bool,
    /// Format-to-series-kind validation rules. Each rule says: "if the
    /// release contains any of these formats, the matched series must
    /// have one of these kinds, otherwise demote to ambiguous". Empty
    /// vector disables format-type validation entirely.
    #[serde(default)]
    pub format_type_rules: Vec<FormatTypeRule>,
}

/// One format-to-kind rule. If the release has at least one of `formats`,
/// the matched series's kind must be in `required_kinds`. Comparison is
/// case-insensitive on both sides.
///
/// Kept as a free-form `String` list of kinds rather than the `SeriesKind`
/// enum so that operators can keep config working when MangaBaka (or a
/// future provider) emits a kind we don't have an enum variant for yet.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FormatTypeRule {
    pub formats: Vec<String>,
    pub required_kinds: Vec<String>,
}

impl Default for IngestionConfig {
    fn default() -> Self {
        Self {
            resolution_threshold: 0.85,
            review_threshold: 0.55,
            fuzzy_search_limit: 10,
            queue_low_confidence: true,
            format_type_rules: Vec::new(),
        }
    }
}

impl Default for NyaaSourceOptions {
    fn default() -> Self {
        Self {
            feed_url: String::new(),
            timeout_seconds: 30,
            fetch_details: true,
            site_base_url: "https://nyaa.si".into(),
        }
    }
}

/// Load configuration, applying defaults, then the base file (if present),
/// then the sibling `.local` overlay (if present), then env vars.
pub fn load(path: &Path) -> anyhow::Result<AppConfig> {
    let mut fig = Figment::from(Serialized::defaults(AppConfig::default()));

    if path.exists() {
        fig = merge_file(fig, path);
    }

    if let Some(local) = local_overlay_path(path)
        && local.exists()
    {
        fig = merge_file(fig, &local);
    }

    fig.merge(Env::prefixed("TSUNDOKU_").split("__"))
        .extract()
        .context("parsing configuration")
}

fn merge_file(fig: Figment, path: &Path) -> Figment {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "yaml" | "yml" => fig.merge(Yaml::file(path)),
        _ => fig.merge(Toml::file(path)),
    }
}

/// Derive the sibling local-overlay path by inserting `.local` before the
/// final extension: `config/tsundoku.docker.toml` -> `config/tsundoku.docker.local.toml`.
/// Returns `None` when the input has no extension to anchor against.
fn local_overlay_path(path: &Path) -> Option<PathBuf> {
    let ext = path.extension()?.to_str()?;
    let stem = path.file_stem()?.to_str()?;
    let mut local = path.to_path_buf();
    local.set_file_name(format!("{stem}.local.{ext}"));
    Some(local)
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
    fn local_overlay_overrides_base_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("tsundoku.docker.toml");
        let local = dir.path().join("tsundoku.docker.local.toml");

        // Base sets a port and one source.
        writeln!(
            std::fs::File::create(&base).unwrap(),
            r#"
[server]
port = 8080

[[sources]]
kind = "nyaa"
name = "broad"
  [sources.nyaa]
  feed_url = "https://nyaa.si/?page=rss&c=3_1&f=2"
            "#
        )
        .unwrap();

        // Local overrides the port AND replaces the sources array (figment
        // replaces arrays wholesale; partial array merges aren't supported).
        writeln!(
            std::fs::File::create(&local).unwrap(),
            r#"
[server]
port = 9000

[[sources]]
kind = "nyaa"
name = "broad"
  [sources.nyaa]
  feed_url = "https://nyaa.si/?page=rss&c=3_1&f=2"

[[sources]]
kind = "nyaa"
name = "uploader-tsuna69"
  [sources.nyaa]
  feed_url = "https://nyaa.si/?page=rss&u=tsuna69"
            "#
        )
        .unwrap();

        let cfg = load(&base).unwrap();
        assert_eq!(cfg.server.port, 9000, "local overlay should win over base");
        assert_eq!(cfg.sources.len(), 2);
        assert_eq!(cfg.sources[1].name, "uploader-tsuna69");
    }

    #[test]
    fn local_overlay_is_optional() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("tsundoku.docker.toml");
        std::fs::write(&base, "[server]\nport = 8081\n").unwrap();

        // No sibling .local.toml exists: load must still succeed.
        let cfg = load(&base).unwrap();
        assert_eq!(cfg.server.port, 8081);
    }

    #[test]
    fn local_overlay_merges_dict_fields_recursively() {
        // Dicts merge field-by-field (only arrays are replaced wholesale), so
        // base + local can each contribute different keys to the same table.
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("tsundoku.docker.toml");
        let local = dir.path().join("tsundoku.docker.local.toml");

        std::fs::write(
            &base,
            r#"
[providers.mangabaka]
api_base_url = "https://proxy.example.com"
timeout_seconds = 90
"#,
        )
        .unwrap();

        std::fs::write(
            &local,
            r#"
[providers.mangabaka]
api_key = "mb-secret"
api_fallback = true
"#,
        )
        .unwrap();

        let cfg = load(&base).unwrap();
        let mb = &cfg.providers.mangabaka;
        // Base-only field survives the merge.
        assert_eq!(mb.api_base_url, "https://proxy.example.com");
        assert_eq!(mb.timeout_seconds, 90);
        // Local-only fields are applied.
        assert_eq!(mb.api_key.as_deref(), Some("mb-secret"));
        assert!(mb.api_fallback);
    }

    #[test]
    fn local_overlay_path_derives_sibling_with_local_infix() {
        // Standard `<stem>.<ext>` cases.
        assert_eq!(
            local_overlay_path(Path::new("config/tsundoku.docker.toml")),
            Some(PathBuf::from("config/tsundoku.docker.local.toml"))
        );
        assert_eq!(
            local_overlay_path(Path::new("config/tsundoku.yaml")),
            Some(PathBuf::from("config/tsundoku.local.yaml"))
        );
        // file_stem() strips only the final extension, so multi-dot stems
        // round-trip cleanly.
        assert_eq!(
            local_overlay_path(Path::new("/etc/tsundoku/app.prod.yml")),
            Some(PathBuf::from("/etc/tsundoku/app.prod.local.yml"))
        );
        // Anchorless paths have no `.local.<ext>` to derive.
        assert_eq!(local_overlay_path(Path::new("tsundoku")), None);
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
        assert!(opts.fetch_details);
        // `enabled` defaults to true when omitted.
        assert!(cfg.sources[1].enabled);
    }

    #[test]
    fn ingestion_block_parses_with_format_type_rules() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tsundoku.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[storage]
data_dir = "./data"

[ingestion]
resolution_threshold = 0.9
review_threshold = 0.6
fuzzy_search_limit = 20
queue_low_confidence = false

[[ingestion.format_type_rules]]
formats = ["cbz", "cbr", "zip"]
required_kinds = ["manga", "manhwa", "manhua"]

[[ingestion.format_type_rules]]
formats = ["epub", "azw3"]
required_kinds = ["novel"]
            "#
        )
        .unwrap();

        let cfg = load(&path).unwrap();
        assert!((cfg.ingestion.resolution_threshold - 0.9).abs() < 1e-6);
        assert!((cfg.ingestion.review_threshold - 0.6).abs() < 1e-6);
        assert_eq!(cfg.ingestion.fuzzy_search_limit, 20);
        assert!(!cfg.ingestion.queue_low_confidence);
        assert_eq!(cfg.ingestion.format_type_rules.len(), 2);
        assert_eq!(
            cfg.ingestion.format_type_rules[0].formats,
            vec!["cbz".to_string(), "cbr".into(), "zip".into()]
        );
        assert_eq!(
            cfg.ingestion.format_type_rules[1].required_kinds,
            vec!["novel".to_string()]
        );
    }

    #[test]
    fn ingestion_defaults_are_conservative() {
        let cfg = load(&PathBuf::from("does-not-exist.toml")).unwrap();
        assert!((cfg.ingestion.resolution_threshold - 0.85).abs() < 1e-6);
        assert!(cfg.ingestion.queue_low_confidence);
        assert!(cfg.ingestion.format_type_rules.is_empty());
    }

    #[test]
    fn auth_defaults_are_open_with_no_tokens() {
        let cfg = load(&PathBuf::from("does-not-exist.toml")).unwrap();
        assert!(!cfg.auth.read_requires_auth);
        assert!(cfg.auth.api_key.is_none());
        assert!(cfg.auth.admin_token.is_none());
    }

    #[test]
    fn auth_block_parses() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tsundoku.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[auth]
read_requires_auth = true
api_key = "read-me"
admin_token = "write-me"
            "#
        )
        .unwrap();
        let cfg = load(&path).unwrap();
        assert!(cfg.auth.read_requires_auth);
        assert_eq!(cfg.auth.api_key.as_deref(), Some("read-me"));
        assert_eq!(cfg.auth.admin_token.as_deref(), Some("write-me"));
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
