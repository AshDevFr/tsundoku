//! Test scaffolding shared by every handler integration test.

use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection};
use td_config::{AppConfig, AuthConfig, IngestionConfig, ProvidersConfig};
use td_metadata::{
    MetadataProvider, MetadataRegistry, MetadataResult, RefreshStatus, RefreshSummary, SearchHit,
    SeriesMetadata,
};
use td_scheduler::JobLocks;
use td_source::{
    DiscoveredRelease, DiscoverySource, PollContext, PollOutcome, SourceRegistry, SourceResult,
};

#[derive(Default)]
pub struct StubProvider {
    pub id: &'static str,
    pub returns: Option<SeriesMetadata>,
    /// Hits returned verbatim by every `search()` call. Tests that
    /// exercise the search endpoint set this; default-built stubs
    /// continue to return empty.
    pub search_hits: Vec<SearchHit>,
    /// Per-external-id metadata. When set, `get()` looks the id up in
    /// this map first (falling back to `returns` on miss). Lets the
    /// search-endpoint tests register multiple candidates with
    /// distinct metadata.
    pub get_table: std::collections::HashMap<String, SeriesMetadata>,
    /// `(foreign_provider, foreign_id) → metadata` for
    /// `resolve_by_foreign_id`. Lets the foreign-id search tests prove the
    /// handler routes a foreign id to cross-resolution rather than `get`.
    pub foreign_table: std::collections::HashMap<(String, String), SeriesMetadata>,
    /// Advertised cross-resolvable providers (drives `foreign_sources()`).
    pub foreign_sources: Vec<&'static str>,
}

#[async_trait]
impl MetadataProvider for StubProvider {
    fn id(&self) -> &str {
        self.id
    }
    fn display_name(&self) -> &str {
        "Stub"
    }
    async fn get(&self, external_id: &str) -> MetadataResult<Option<SeriesMetadata>> {
        if let Some(m) = self.get_table.get(external_id) {
            return Ok(Some(m.clone()));
        }
        Ok(self.returns.clone())
    }
    async fn search(&self, _query: &str, _limit: u32) -> MetadataResult<Vec<SearchHit>> {
        Ok(self.search_hits.clone())
    }
    async fn resolve_by_foreign_id(
        &self,
        foreign_provider: &str,
        foreign_id: &str,
    ) -> MetadataResult<Option<SeriesMetadata>> {
        Ok(self
            .foreign_table
            .get(&(foreign_provider.to_string(), foreign_id.to_string()))
            .cloned())
    }
    fn foreign_sources(&self) -> &'static [&'static str] {
        // Leak once per stub: tests are short-lived and this keeps the
        // trait's `&'static` contract without a global.
        Box::leak(self.foreign_sources.clone().into_boxed_slice())
    }
    async fn refresh_cache(&self) -> MetadataResult<RefreshSummary> {
        let now = chrono::Utc::now();
        Ok(RefreshSummary {
            provider: self.id.into(),
            status: RefreshStatus::Refreshed {
                records: 0,
                version: Some("v0".into()),
            },
            started_at: now,
            finished_at: now,
            bytes_downloaded: Some(0),
        })
    }
}

pub struct StubSource {
    pub name: String,
    pub kind: String,
    pub outcome: PollOutcome,
}

#[async_trait]
impl DiscoverySource for StubSource {
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> &str {
        &self.kind
    }
    async fn poll(&self, _ctx: &PollContext) -> SourceResult<PollOutcome> {
        Ok(self.outcome.clone())
    }
}

pub async fn fresh_db() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

pub fn metadata_registry_with(stub: StubProvider) -> MetadataRegistry {
    let id = stub.id.to_string();
    let mut b = MetadataRegistry::builder();
    b.register(Arc::new(stub) as Arc<dyn MetadataProvider>)
        .unwrap();
    b.set_active(id);
    b.build().unwrap()
}

pub fn source_registry_with(stubs: Vec<StubSource>) -> SourceRegistry {
    let mut b = SourceRegistry::builder();
    for s in stubs {
        b.register(Arc::new(s) as Arc<dyn DiscoverySource>).unwrap();
    }
    b.build()
}

pub fn build_app(
    db: DatabaseConnection,
    metadata: MetadataRegistry,
    sources: SourceRegistry,
    auth: AuthConfig,
) -> Router {
    build_app_full(
        db,
        metadata,
        sources,
        auth,
        Vec::new(),
        ProvidersConfig::default(),
        Arc::new(JobLocks::default()),
    )
}

/// Variant used by tests that need to inject `[[sources]]` / `[providers]`
/// config blocks or a pre-built `JobLocks` (to simulate an in-flight tick).
pub fn build_app_full(
    db: DatabaseConnection,
    metadata: MetadataRegistry,
    sources: SourceRegistry,
    auth: AuthConfig,
    sources_config: Vec<td_config::SourceConfig>,
    providers_config: ProvidersConfig,
    locks: Arc<JobLocks>,
) -> Router {
    let (app, _) = build_app_with_events(
        db,
        metadata,
        sources,
        auth,
        sources_config,
        providers_config,
        locks,
    );
    app
}

/// Like [`build_app_full`] but also returns a sender clone for the
/// job-event broadcast channel. Tests can call `.subscribe()` on the
/// sender to read events the handlers publish.
pub fn build_app_with_events(
    db: DatabaseConnection,
    metadata: MetadataRegistry,
    sources: SourceRegistry,
    auth: AuthConfig,
    sources_config: Vec<td_config::SourceConfig>,
    providers_config: ProvidersConfig,
    locks: Arc<JobLocks>,
) -> (Router, tokio::sync::broadcast::Sender<td_api::JobEvent>) {
    let cfg = AppConfig {
        auth: auth.clone(),
        api: td_config::ApiConfig { docs: false },
        ..AppConfig::default()
    };
    let (job_events, _) = tokio::sync::broadcast::channel(td_api::JOB_EVENT_BUFFER);
    let events_handle = job_events.clone();
    let state = td_api::AppState {
        db,
        sources: Arc::new(sources),
        metadata: Arc::new(metadata),
        ingestion: IngestionConfig::default(),
        auth: Arc::new(auth),
        locks,
        sources_config: Arc::new(sources_config),
        search: Arc::new(td_source::SearchRegistry::builder().build()),
        search_config: Arc::new(Vec::new()),
        providers_config: Arc::new(providers_config),
        metadata_config: Arc::new(td_config::MetadataConfig::default()),
        query_builder: Arc::new(td_resolution::query_builder::QueryBuilder::with_defaults()),
        mangaupdates_redirector: None,
        job_events,
        cover_cache_dir: None,
        codex: Arc::new(td_config::CodexConfig::default()),
        codex_client: None,
        download: Arc::new(td_config::DownloadConfig::default()),
        download_client: None,
    };
    (td_api::router(state, &cfg), events_handle)
}

/// Router variant for the cover-proxy tests. Wires `cover_cache_dir`
/// (the only field that matters for `/api/v1/covers/*`) and leaves every
/// other knob at the same defaults as [`build_app`].
pub fn build_app_with_cover_cache(
    db: DatabaseConnection,
    auth: AuthConfig,
    cover_cache_dir: std::path::PathBuf,
) -> Router {
    let metadata = metadata_registry_with(StubProvider {
        id: "stub",
        ..Default::default()
    });
    let sources = source_registry_with(vec![]);
    let cfg = AppConfig {
        auth: auth.clone(),
        api: td_config::ApiConfig { docs: false },
        ..AppConfig::default()
    };
    let (job_events, _) = tokio::sync::broadcast::channel(td_api::JOB_EVENT_BUFFER);
    let state = td_api::AppState {
        db,
        sources: Arc::new(sources),
        metadata: Arc::new(metadata),
        ingestion: IngestionConfig::default(),
        auth: Arc::new(auth),
        locks: Arc::new(JobLocks::default()),
        sources_config: Arc::new(Vec::new()),
        search: Arc::new(td_source::SearchRegistry::builder().build()),
        search_config: Arc::new(Vec::new()),
        providers_config: Arc::new(ProvidersConfig::default()),
        metadata_config: Arc::new(td_config::MetadataConfig::default()),
        query_builder: Arc::new(td_resolution::query_builder::QueryBuilder::with_defaults()),
        mangaupdates_redirector: None,
        job_events,
        cover_cache_dir: Some(cover_cache_dir),
        codex: Arc::new(td_config::CodexConfig::default()),
        codex_client: None,
        download: Arc::new(td_config::DownloadConfig::default()),
        download_client: None,
    };
    td_api::router(state, &cfg)
}

/// Router for the codex-handler tests. `codex` sets the `[codex]` config
/// snapshot (the `enabled` flag the status endpoint reads); `codex_client` is
/// the optional client the refresh trigger needs; `locks` lets a test pre-hold
/// the codex lock to assert the skipped path.
pub fn build_app_with_codex(
    db: DatabaseConnection,
    auth: AuthConfig,
    codex: td_config::CodexConfig,
    codex_client: Option<Arc<td_codex::CodexClient>>,
    locks: Arc<JobLocks>,
) -> Router {
    let cfg = AppConfig {
        auth: auth.clone(),
        api: td_config::ApiConfig { docs: false },
        ..AppConfig::default()
    };
    let (job_events, _) = tokio::sync::broadcast::channel(td_api::JOB_EVENT_BUFFER);
    let metadata = metadata_registry_with(StubProvider {
        id: "stub",
        ..Default::default()
    });
    let sources = source_registry_with(vec![]);
    let state = td_api::AppState {
        db,
        sources: Arc::new(sources),
        metadata: Arc::new(metadata),
        ingestion: IngestionConfig::default(),
        auth: Arc::new(auth),
        locks,
        sources_config: Arc::new(Vec::new()),
        search: Arc::new(td_source::SearchRegistry::builder().build()),
        search_config: Arc::new(Vec::new()),
        providers_config: Arc::new(ProvidersConfig::default()),
        metadata_config: Arc::new(td_config::MetadataConfig::default()),
        query_builder: Arc::new(td_resolution::query_builder::QueryBuilder::with_defaults()),
        mangaupdates_redirector: None,
        job_events,
        cover_cache_dir: None,
        codex: Arc::new(codex),
        codex_client,
        download: Arc::new(td_config::DownloadConfig::default()),
        download_client: None,
    };
    td_api::router(state, &cfg)
}

/// A `CodexClient` pointed at an unreachable address. Enough to give the
/// refresh trigger a `Some(client)` so it dispatches; the background sweep
/// fails harmlessly after the HTTP response is already sent.
pub fn unreachable_codex_client() -> Arc<td_codex::CodexClient> {
    Arc::new(
        td_codex::CodexClient::new(
            "http://127.0.0.1:9",
            "test-key",
            std::time::Duration::from_millis(50),
            td_http::HttpLimiter::no_limit(),
        )
        .unwrap(),
    )
}

/// Router variant for the send-to-client tests. `download` sets the
/// `[download]` config snapshot (the `enabled`/`kind` the status endpoint
/// reads and the per-send defaults the send handler applies); `download_client`
/// is the optional client the send endpoint needs (`None` ⇒ 503).
pub fn build_app_with_download(
    db: DatabaseConnection,
    auth: AuthConfig,
    download: td_config::DownloadConfig,
    download_client: Option<Arc<dyn td_download::DownloadClient>>,
) -> Router {
    let cfg = AppConfig {
        auth: auth.clone(),
        api: td_config::ApiConfig { docs: false },
        ..AppConfig::default()
    };
    let (job_events, _) = tokio::sync::broadcast::channel(td_api::JOB_EVENT_BUFFER);
    let metadata = metadata_registry_with(StubProvider {
        id: "stub",
        ..Default::default()
    });
    let sources = source_registry_with(vec![]);
    let state = td_api::AppState {
        db,
        sources: Arc::new(sources),
        metadata: Arc::new(metadata),
        ingestion: IngestionConfig::default(),
        auth: Arc::new(auth),
        locks: Arc::new(JobLocks::default()),
        sources_config: Arc::new(Vec::new()),
        search: Arc::new(td_source::SearchRegistry::builder().build()),
        search_config: Arc::new(Vec::new()),
        providers_config: Arc::new(ProvidersConfig::default()),
        metadata_config: Arc::new(td_config::MetadataConfig::default()),
        query_builder: Arc::new(td_resolution::query_builder::QueryBuilder::with_defaults()),
        mangaupdates_redirector: None,
        job_events,
        cover_cache_dir: None,
        codex: Arc::new(td_config::CodexConfig::default()),
        codex_client: None,
        download: Arc::new(download),
        download_client,
    };
    td_api::router(state, &cfg)
}

/// A download client pointed at an unreachable address. Gives the send
/// endpoint a `Some(client)` so it gets past the 503 guard; the guard-path
/// tests (404 / 400) return before any outbound request is made.
pub fn unreachable_download_client() -> Arc<dyn td_download::DownloadClient> {
    Arc::new(
        td_download::RtorrentXmlRpcClient::new(
            "http://127.0.0.1:9",
            None,
            None,
            None,
            std::time::Duration::from_millis(50),
            td_http::HttpLimiter::no_limit(),
        )
        .unwrap(),
    )
}

/// Release-search endpoint double for the search-handler tests. Returns
/// `hits` on page 1 of every query and empty afterwards; `delay` lets the
/// skipped-path test hold the per-entry lock long enough to collide.
pub struct StubSearchSource {
    pub name: String,
    pub hits: Vec<DiscoveredRelease>,
    pub delay: Option<std::time::Duration>,
}

#[async_trait]
impl td_source::SearchSource for StubSearchSource {
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> &str {
        "test"
    }
    async fn search_page(&self, _query: &str, page: u32) -> SourceResult<Vec<DiscoveredRelease>> {
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        Ok(if page == 1 {
            self.hits.clone()
        } else {
            Vec::new()
        })
    }
}

/// Router variant for the search-handler tests: builds the search registry
/// from `(source, is_default)` pairs and mirrors each into a
/// `SearchEntryConfig` snapshot (nyaa options block with a recognizable
/// URL) so the entries endpoint has display fields to surface. `locks`
/// lets a test pre-hold a `search:<name>` lock to assert the skipped path.
pub fn build_app_with_search(
    db: DatabaseConnection,
    auth: AuthConfig,
    entries: Vec<(StubSearchSource, bool)>,
    locks: Arc<JobLocks>,
) -> Router {
    let cfg = AppConfig {
        auth: auth.clone(),
        api: td_config::ApiConfig { docs: false },
        ..AppConfig::default()
    };
    let (job_events, _) = tokio::sync::broadcast::channel(td_api::JOB_EVENT_BUFFER);
    let metadata = metadata_registry_with(StubProvider {
        id: "stub",
        ..Default::default()
    });
    let sources = source_registry_with(vec![]);
    let mut builder = td_source::SearchRegistry::builder();
    let mut search_config = Vec::new();
    for (source, is_default) in entries {
        search_config.push(td_config::SearchEntryConfig {
            kind: "nyaa".into(),
            name: source.name.clone(),
            is_default,
            enabled: true,
            max_pages: 3,
            nyaa: Some(td_config::NyaaSearchOptions {
                search_url: format!("https://nyaa.test/?c=3_1&entry={}", source.name),
                ..Default::default()
            }),
        });
        builder
            .register(td_source::SearchEntry {
                source: Arc::new(source),
                is_default,
                max_pages: 3,
            })
            .unwrap();
    }
    let state = td_api::AppState {
        db,
        sources: Arc::new(sources),
        metadata: Arc::new(metadata),
        ingestion: IngestionConfig::default(),
        auth: Arc::new(auth),
        locks,
        sources_config: Arc::new(Vec::new()),
        search: Arc::new(builder.build()),
        search_config: Arc::new(search_config),
        providers_config: Arc::new(ProvidersConfig::default()),
        metadata_config: Arc::new(td_config::MetadataConfig::default()),
        query_builder: Arc::new(td_resolution::query_builder::QueryBuilder::with_defaults()),
        mangaupdates_redirector: None,
        job_events,
        cover_cache_dir: None,
        codex: Arc::new(td_config::CodexConfig::default()),
        codex_client: None,
        download: Arc::new(td_config::DownloadConfig::default()),
        download_client: None,
    };
    td_api::router(state, &cfg)
}

pub fn sample_release(id: &str, source_name: &str, title: &str) -> DiscoveredRelease {
    DiscoveredRelease {
        source_kind: "test".into(),
        source_name: source_name.into(),
        external_id: id.into(),
        title: title.into(),
        link: format!("https://example.com/{id}"),
        magnet: None,
        torrent_url: None,
        ddl_url: None,
        info_hash: None,
        size_bytes: None,
        files: vec![],
        description_html: None,
        external_links: Default::default(),
        comment_suggested_links: Default::default(),
        information_url: None,
        posted_at: chrono::Utc::now(),
    }
}

pub fn sample_metadata(provider: &str, id: &str, title: &str) -> SeriesMetadata {
    SeriesMetadata {
        external_id: id.into(),
        canonical_title: title.into(),
        alternate_titles: vec![],
        kind: Some(td_metadata::SeriesKind::Manga),
        status: Some(td_metadata::SeriesStatus::Ongoing),
        year: Some(2020),
        cover_url: None,
        total_volumes: Some(7),
        total_chapters: Some(42),
        rating: Some(8.0),
        description: None,
        genres: vec![],
        tags: vec![],
        foreign_ids: vec![],
        raw: serde_json::json!({"provider": provider, "id": id, "title": title}),
        published_start_date: None,
        published_end_date: None,
        content_hash: format!("hash-{id}"),
    }
}
