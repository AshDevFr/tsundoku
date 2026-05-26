//! Scheduler tests.
//!
//! We exercise both the trait-level tick functions (`poll_source::run_tick`
//! and `refresh_provider_cache::run_tick`) directly — they're the unit of
//! work — and one timing-sensitive integration test that drives a real
//! [`Scheduler`] with a fast cron to prove the cron wiring works.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection, EntityTrait};
use td_config::{
    AppConfig, IngestionConfig, MangabakaProviderConfig, MetadataConfig, NyaaSourceOptions,
    ProvidersConfig, SourceConfig,
};
use td_db::repos::{provider_cache_state_repo, sources_repo};
use td_metadata::{
    MetadataProvider, MetadataRegistry, MetadataResult, RefreshStatus, RefreshSummary, SearchHit,
    SeriesMetadata,
};
use td_resolution::ResolutionPath;
use td_scheduler::{JobLocks, Scheduler, SchedulerContext, jobs};
use td_source::{
    DiscoveredRelease, DiscoverySource, PollContext, PollOutcome, SourceRegistry, SourceResult,
};

// -----------------------------------------------------------------------------
// Fakes
// -----------------------------------------------------------------------------

/// Discovery-source double. Returns a fixed `PollOutcome` and records the
/// number of `poll()` invocations.
struct FakeSource {
    name: String,
    kind: String,
    outcome: PollOutcome,
    poll_count: AtomicUsize,
}

impl FakeSource {
    fn new(name: &str, kind: &str, outcome: PollOutcome) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            outcome,
            poll_count: AtomicUsize::new(0),
        }
    }

    fn polls(&self) -> usize {
        self.poll_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl DiscoverySource for FakeSource {
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> &str {
        &self.kind
    }
    async fn poll(&self, _ctx: &PollContext) -> SourceResult<PollOutcome> {
        self.poll_count.fetch_add(1, Ordering::SeqCst);
        Ok(self.outcome.clone())
    }
}

/// Metadata-provider double. `search`/`get` return empty so the resolver
/// lands a release at `unresolved` without exercising the persistence
/// pipeline (already covered by td-resolution's own tests). `refresh_cache`
/// records its call count and returns whatever status the test configures.
struct FakeProvider {
    id: String,
    refresh_count: AtomicUsize,
    status: RefreshStatus,
}

impl FakeProvider {
    fn new(id: &str, status: RefreshStatus) -> Self {
        Self {
            id: id.into(),
            refresh_count: AtomicUsize::new(0),
            status,
        }
    }

    fn refreshes(&self) -> usize {
        self.refresh_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl MetadataProvider for FakeProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn display_name(&self) -> &str {
        "Fake"
    }
    async fn get(&self, _external_id: &str) -> MetadataResult<Option<SeriesMetadata>> {
        Ok(None)
    }
    async fn search(&self, _query: &str, _limit: u32) -> MetadataResult<Vec<SearchHit>> {
        Ok(Vec::new())
    }
    async fn refresh_cache(&self) -> MetadataResult<RefreshSummary> {
        self.refresh_count.fetch_add(1, Ordering::SeqCst);
        let now = Utc::now();
        Ok(RefreshSummary {
            provider: self.id.clone(),
            status: self.status.clone(),
            started_at: now,
            finished_at: now,
            bytes_downloaded: match &self.status {
                RefreshStatus::Refreshed { .. } => Some(42),
                _ => None,
            },
        })
    }
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

async fn fresh_db() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

fn discovered_release(source_name: &str, external_id: &str, title: &str) -> DiscoveredRelease {
    DiscoveredRelease {
        source_kind: "fake".into(),
        source_name: source_name.into(),
        external_id: external_id.into(),
        title: title.into(),
        link: format!("https://example.test/{external_id}"),
        magnet: None,
        torrent_url: None,
        ddl_url: None,
        info_hash: None,
        size_bytes: None,
        files: vec![format!("{title}.cbz")],
        description_html: None,
        external_links: Default::default(),
        posted_at: Utc::now(),
    }
}

fn build_registry(provider: Arc<FakeProvider>) -> Arc<MetadataRegistry> {
    let id = provider.id().to_string();
    let mut builder = MetadataRegistry::builder();
    builder
        .register(provider as Arc<dyn MetadataProvider>)
        .unwrap();
    builder.set_active(id);
    Arc::new(builder.build().unwrap())
}

// -----------------------------------------------------------------------------
// poll_source::run_tick
// -----------------------------------------------------------------------------

#[tokio::test]
async fn poll_tick_persists_releases_and_updates_source_state() {
    let db = fresh_db().await;

    let releases = vec![
        discovered_release("trusted", "ext-1", "Chainsaw Man"),
        discovered_release("trusted", "ext-2", "Spy x Family"),
    ];
    let source = Arc::new(FakeSource::new(
        "trusted",
        "fake",
        PollOutcome {
            releases,
            new_etag: Some("etag-xyz".into()),
            new_cursor: None,
            not_modified: false,
        },
    ));
    let provider = Arc::new(FakeProvider::new("mangabaka", RefreshStatus::NotSupported));
    let metadata = build_registry(provider);

    let locks = Arc::new(JobLocks::default());
    jobs::poll_source::run_tick(
        source.clone() as Arc<dyn DiscoverySource>,
        db.clone(),
        metadata,
        IngestionConfig::default(),
        locks,
        "cron",
    )
    .await;

    assert_eq!(source.polls(), 1);

    // Both releases land in storage.
    let rows = td_db::entities::releases::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.resolution_status == "unresolved"));

    // source_state was upserted with the new ETag + a summary.
    let state = sources_repo::get(&db, "fake", "trusted").await.unwrap();
    let state = state.expect("source_state row should exist after a successful tick");
    assert_eq!(state.etag.as_deref(), Some("etag-xyz"));
    assert!(state.last_success_at.is_some());
    let summary = state.last_summary.unwrap_or_default();
    assert!(summary.contains("2 fetched"), "got {summary:?}");
    assert!(summary.contains("2 persisted"), "got {summary:?}");

    // The tick also records exactly one poll_runs row with status=success.
    let runs = td_db::entities::poll_runs::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.source_name, "trusted");
    assert_eq!(run.source_kind, "fake");
    assert_eq!(run.status, "success");
    assert_eq!(run.fetched_count, Some(2));
    assert_eq!(run.new_count, Some(2));
    assert!(run.finished_at.is_some());
    assert_eq!(run.trigger, "cron");
}

#[tokio::test]
async fn poll_tick_with_held_lock_is_a_noop() {
    let db = fresh_db().await;
    let source = Arc::new(FakeSource::new(
        "trusted",
        "fake",
        PollOutcome::from_releases(vec![discovered_release("trusted", "ext-1", "Chainsaw Man")]),
    ));
    let provider = Arc::new(FakeProvider::new("mangabaka", RefreshStatus::NotSupported));
    let metadata = build_registry(provider);

    let locks = Arc::new(JobLocks::default());

    // Hold the per-source lock so the tick should bail immediately.
    let lock = locks.source_lock("trusted");
    let _guard = lock.lock().await;

    jobs::poll_source::run_tick(
        source.clone() as Arc<dyn DiscoverySource>,
        db.clone(),
        metadata,
        IngestionConfig::default(),
        locks.clone(),
        "cron",
    )
    .await;

    assert_eq!(
        source.polls(),
        0,
        "tick should have skipped while lock was held"
    );
    let count = td_db::entities::releases::Entity::find()
        .all(&db)
        .await
        .unwrap()
        .len();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn poll_tick_handles_source_failure_without_panicking() {
    struct FailingSource;

    #[async_trait]
    impl DiscoverySource for FailingSource {
        fn name(&self) -> &str {
            "trusted"
        }
        fn kind(&self) -> &str {
            "fake"
        }
        async fn poll(&self, _ctx: &PollContext) -> SourceResult<PollOutcome> {
            Err(td_source::SourceError::Other(anyhow::anyhow!(
                "simulated upstream outage"
            )))
        }
    }

    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mangabaka", RefreshStatus::NotSupported));
    let metadata = build_registry(provider);
    let locks = Arc::new(JobLocks::default());

    jobs::poll_source::run_tick(
        Arc::new(FailingSource),
        db.clone(),
        metadata,
        IngestionConfig::default(),
        locks,
        "cron",
    )
    .await;

    // No releases, but source_state.last_error captured the failure.
    let state = sources_repo::get(&db, "fake", "trusted")
        .await
        .unwrap()
        .expect("source_state row should still be written on failure");
    assert!(state.last_error.is_some());
    let summary = state.last_summary.unwrap_or_default();
    assert!(summary.starts_with("error:"), "got {summary:?}");
}

// -----------------------------------------------------------------------------
// refresh_provider_cache::run_tick
// -----------------------------------------------------------------------------

#[tokio::test]
async fn refresh_tick_appends_provider_cache_state_on_refreshed_status() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new(
        "mangabaka",
        RefreshStatus::Refreshed {
            records: 12345,
            version: Some("v1".into()),
        },
    ));
    let locks = Arc::new(JobLocks::default());

    jobs::refresh_provider_cache::run_tick(
        provider.clone() as Arc<dyn MetadataProvider>,
        db.clone(),
        locks,
        "cron",
    )
    .await;

    assert_eq!(provider.refreshes(), 1);
    let latest = provider_cache_state_repo::latest(&db, "mangabaka")
        .await
        .unwrap()
        .expect("expected a provider_cache_state row after a Refreshed status");
    assert_eq!(latest.record_count, Some(12345));
    assert_eq!(latest.cache_version.as_deref(), Some("v1"));
    assert_eq!(latest.bytes_downloaded, Some(42));

    // Refresh metrics row is also written.
    let refreshes = td_db::entities::provider_refreshes::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(refreshes.len(), 1);
    assert_eq!(refreshes[0].status, "success");
    assert_eq!(refreshes[0].record_count, Some(12345));
    assert_eq!(refreshes[0].trigger, "cron");
}

#[tokio::test]
async fn refresh_tick_skips_persistence_for_not_supported() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mangabaka", RefreshStatus::NotSupported));
    let locks = Arc::new(JobLocks::default());

    jobs::refresh_provider_cache::run_tick(
        provider.clone() as Arc<dyn MetadataProvider>,
        db.clone(),
        locks,
        "cron",
    )
    .await;

    // Provider was called but no row was written (NotSupported is informational).
    assert_eq!(provider.refreshes(), 1);
    let latest = provider_cache_state_repo::latest(&db, "mangabaka")
        .await
        .unwrap();
    assert!(latest.is_none());
}

#[tokio::test]
async fn refresh_tick_with_held_lock_is_a_noop() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new(
        "mangabaka",
        RefreshStatus::Refreshed {
            records: 1,
            version: None,
        },
    ));
    let locks = Arc::new(JobLocks::default());

    let lock = locks.provider_lock("mangabaka");
    let _guard = lock.lock().await;

    jobs::refresh_provider_cache::run_tick(
        provider.clone() as Arc<dyn MetadataProvider>,
        db.clone(),
        locks.clone(),
        "cron",
    )
    .await;

    assert_eq!(provider.refreshes(), 0);
    let latest = provider_cache_state_repo::latest(&db, "mangabaka")
        .await
        .unwrap();
    assert!(latest.is_none());
}

// -----------------------------------------------------------------------------
// End-to-end: a real Scheduler driving a real cron schedule.
// -----------------------------------------------------------------------------

/// Verify that a [`Scheduler`] built from `AppConfig` actually fires jobs on
/// cron ticks: register a `*/1 * * * * *` (every second) job, wait ~2.5
/// seconds, expect at least 2 invocations.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduler_fires_source_and_provider_jobs_on_schedule() {
    let db = fresh_db().await;

    // Source with no releases (keeps the tick cheap).
    let source = Arc::new(FakeSource::new("trusted", "fake", PollOutcome::default()));
    let mut sources_builder = SourceRegistry::builder();
    sources_builder
        .register(source.clone() as Arc<dyn DiscoverySource>)
        .unwrap();
    let sources = Arc::new(sources_builder.build());

    let provider = Arc::new(FakeProvider::new(
        "mangabaka",
        RefreshStatus::Refreshed {
            records: 1,
            version: None,
        },
    ));
    let metadata = build_registry(provider.clone());

    let cfg = AppConfig {
        metadata: MetadataConfig {
            active_provider: "mangabaka".into(),
        },
        providers: ProvidersConfig {
            mangabaka: MangabakaProviderConfig {
                offline_refresh_cron: Some("*/1 * * * * *".into()),
                ..MangabakaProviderConfig::default()
            },
        },
        sources: vec![SourceConfig {
            kind: "fake".into(),
            name: "trusted".into(),
            cron: Some("*/1 * * * * *".into()),
            enabled: true,
            nyaa: Some(NyaaSourceOptions::default()),
        }],
        ..Default::default()
    };

    let ctx = SchedulerContext {
        db: db.clone(),
        sources,
        metadata,
        ingestion: IngestionConfig::default(),
        locks: Arc::new(JobLocks::default()),
    };

    let mut scheduler = Scheduler::build(&cfg, ctx).await.unwrap();
    // 1 source job + 1 provider refresh job + 1 review-queue snapshot job.
    assert_eq!(scheduler.job_count(), 3);
    scheduler.start().await.unwrap();

    tokio::time::sleep(Duration::from_millis(2500)).await;

    scheduler.shutdown().await.unwrap();

    assert!(
        source.polls() >= 2,
        "expected at least 2 polls within ~2.5s, got {}",
        source.polls()
    );
    assert!(
        provider.refreshes() >= 2,
        "expected at least 2 refreshes within ~2.5s, got {}",
        provider.refreshes()
    );
}

/// Sanity check: sources without a cron are skipped, and unregistered
/// sources are skipped with a warning. Neither raises an error.
#[tokio::test]
async fn scheduler_skips_sources_without_cron_or_unknown_registry_entry() {
    let db = fresh_db().await;

    // Registry contains "trusted" but cfg.sources references "other".
    let source = Arc::new(FakeSource::new("trusted", "fake", PollOutcome::default()));
    let mut sources_builder = SourceRegistry::builder();
    sources_builder
        .register(source.clone() as Arc<dyn DiscoverySource>)
        .unwrap();
    let sources = Arc::new(sources_builder.build());

    let provider = Arc::new(FakeProvider::new("mangabaka", RefreshStatus::NotSupported));
    let metadata = build_registry(provider);

    let cfg = AppConfig {
        providers: ProvidersConfig {
            mangabaka: MangabakaProviderConfig {
                offline_refresh_cron: None, // provider job: skipped (no cron)
                ..MangabakaProviderConfig::default()
            },
        },
        sources: vec![
            SourceConfig {
                kind: "fake".into(),
                name: "trusted".into(),
                cron: None, // skipped: no cron
                enabled: true,
                nyaa: None,
            },
            SourceConfig {
                kind: "fake".into(),
                name: "other".into(),
                cron: Some("*/5 * * * *".into()), // not in the registry: skipped
                enabled: true,
                nyaa: None,
            },
        ],
        ..Default::default()
    };

    let ctx = SchedulerContext {
        db: db.clone(),
        sources,
        metadata,
        ingestion: IngestionConfig::default(),
        locks: Arc::new(JobLocks::default()),
    };
    let scheduler = Scheduler::build(&cfg, ctx).await.unwrap();
    // Only the unconditional review-queue snapshot job lands; source +
    // provider jobs are skipped.
    assert_eq!(scheduler.job_count(), 1);
}

#[tokio::test]
async fn snapshot_review_queue_writes_row_with_pending_breakdown() {
    use sea_orm::{ActiveModelTrait, Set};
    use td_db::entities::releases as releases_entity;

    let db = fresh_db().await;
    // Seed two pending releases: one unresolved (older), one ambiguous.
    let now = chrono::Utc::now().timestamp();
    let pending = releases_entity::ActiveModel {
        id: Set("nyaa:p:1".into()),
        source_kind: Set("nyaa".into()),
        source_name: Set("trusted".into()),
        external_id: Set("1".into()),
        title: Set("Pending".into()),
        link: Set("https://example.com/1".into()),
        magnet: Set(None),
        torrent_url: Set(None),
        ddl_url: Set(None),
        info_hash: Set(None),
        size_bytes: Set(None),
        files_json: Set(None),
        description_html: Set(None),
        extracted_links_json: Set(None),
        posted_at: Set(now - 7_200),
        observed_at: Set(now - 7_200),
        series_id: Set(None),
        resolution_path: Set(None),
        resolution_confidence: Set(None),
        resolution_status: Set("unresolved".into()),
        resolution_attempts: Set(1),
        last_resolve_attempt_at: Set(None),
        volume_span_json: Set(None),
        chapter_span_json: Set(None),
        resolved_at: Set(None),
    };
    pending.insert(&db).await.unwrap();
    let ambiguous = releases_entity::ActiveModel {
        id: Set("nyaa:p:2".into()),
        source_kind: Set("nyaa".into()),
        source_name: Set("trusted".into()),
        external_id: Set("2".into()),
        title: Set("Ambig".into()),
        link: Set("https://example.com/2".into()),
        magnet: Set(None),
        torrent_url: Set(None),
        ddl_url: Set(None),
        info_hash: Set(None),
        size_bytes: Set(None),
        files_json: Set(None),
        description_html: Set(None),
        extracted_links_json: Set(None),
        posted_at: Set(now - 300),
        observed_at: Set(now - 300),
        series_id: Set(None),
        resolution_path: Set(None),
        resolution_confidence: Set(None),
        resolution_status: Set("ambiguous".into()),
        resolution_attempts: Set(1),
        last_resolve_attempt_at: Set(None),
        volume_span_json: Set(None),
        chapter_span_json: Set(None),
        resolved_at: Set(None),
    };
    ambiguous.insert(&db).await.unwrap();

    jobs::snapshot_review_queue::run_tick(db.clone()).await;

    let rows = td_db::entities::review_queue_snapshots::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.unresolved_count, 1);
    assert_eq!(row.ambiguous_count, 1);
    assert_eq!(row.review_pending_count, 0);
    assert_eq!(row.pending_count, 2);
    // Oldest pending is ~2 hours old.
    let age = row.oldest_pending_seconds.unwrap();
    assert!((7_000..=7_500).contains(&age), "got age {age}");
}

#[test]
fn resolution_path_export_compiles() {
    // Pulls in `td_resolution::ResolutionPath` to confirm the dependency
    // graph picks up the resolver crate (which the poll tick uses).
    let _ = ResolutionPath::KnownExternalId;
}
