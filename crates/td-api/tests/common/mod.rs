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
        providers_config: Arc::new(providers_config),
        query_builder: Arc::new(td_resolution::query_builder::QueryBuilder::with_defaults()),
        mangaupdates_redirector: None,
        job_events,
    };
    (td_api::router(state, &cfg), events_handle)
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
        content_hash: format!("hash-{id}"),
    }
}
