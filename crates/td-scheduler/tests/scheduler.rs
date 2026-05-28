//! Scheduler tests.
//!
//! We exercise both the trait-level tick functions (`poll_source::run_tick`
//! and `refresh_provider_cache::run_tick`) directly — they're the unit of
//! work — and one timing-sensitive integration test that drives a real
//! [`Scheduler`] with a fast cron to prove the cron wiring works.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection, EntityTrait};
use td_config::{
    AppConfig, IngestionConfig, MangabakaProviderConfig, MetadataConfig, NyaaSourceOptions,
    ProvidersConfig, SeriesRefreshConfig, SourceConfig,
};
use td_db::repos::{provider_cache_state_repo, sources_repo};
use td_metadata::{
    MetadataError, MetadataProvider, MetadataRegistry, MetadataResult, RefreshStatus,
    RefreshSummary, SearchHit, SeriesMetadata,
};
use td_resolution::ResolutionPath;
use td_scheduler::{JobLocks, Scheduler, SchedulerContext, jobs};
use tokio::sync::broadcast;

/// Tests don't subscribe to the SSE channel; this helper returns a
/// detached sender to fill the `job_events` field on `SchedulerContext`.
/// `send()` returning `Err` (no receivers) is fine — every emit site
/// already ignores the error.
fn detached_events() -> broadcast::Sender<td_scheduler::JobEvent> {
    broadcast::channel(16).0
}
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

/// One scripted outcome for `MetadataProvider::get`. Build the
/// `FakeProvider` with `.with_get` to map an external_id to one of these
/// variants; unmapped IDs fall back to `Ok(None)` (matching the original
/// FakeProvider behaviour so the older tests keep working).
enum GetOutcome {
    Some(Box<SeriesMetadata>),
    NotFound,
    Err(String),
}

/// Metadata-provider double. `refresh_cache` records its call count and
/// returns whatever status the test configures. `get` consults the
/// scripted map (defaults to `Ok(None)` when empty or unmapped).
struct FakeProvider {
    id: String,
    refresh_count: AtomicUsize,
    get_count: AtomicUsize,
    status: RefreshStatus,
    /// `external_id → outcome`. Wrapped in a std `Mutex` because the
    /// trait method is `&self` and the map is only mutated from test
    /// setup before any async work starts.
    get_map: StdMutex<HashMap<String, GetOutcome>>,
}

impl FakeProvider {
    fn new(id: &str, status: RefreshStatus) -> Self {
        Self {
            id: id.into(),
            refresh_count: AtomicUsize::new(0),
            get_count: AtomicUsize::new(0),
            status,
            get_map: StdMutex::new(HashMap::new()),
        }
    }

    fn refreshes(&self) -> usize {
        self.refresh_count.load(Ordering::SeqCst)
    }

    fn gets(&self) -> usize {
        self.get_count.load(Ordering::SeqCst)
    }

    /// Script the response for one external_id. Repeat to add more.
    fn with_get(self, external_id: &str, outcome: GetOutcome) -> Self {
        self.get_map
            .lock()
            .unwrap()
            .insert(external_id.into(), outcome);
        self
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
    async fn get(&self, external_id: &str) -> MetadataResult<Option<SeriesMetadata>> {
        self.get_count.fetch_add(1, Ordering::SeqCst);
        let outcome = self.get_map.lock().unwrap().remove(external_id);
        match outcome {
            Some(GetOutcome::Some(m)) => Ok(Some(*m)),
            Some(GetOutcome::NotFound) => Ok(None),
            Some(GetOutcome::Err(msg)) => Err(MetadataError::Unavailable {
                provider: self.id.clone(),
                source: anyhow::anyhow!(msg),
            }),
            None => Ok(None),
        }
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
        Arc::new(td_resolution::query_builder::QueryBuilder::with_defaults()),
        None,
        detached_events(),
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
    // Progress columns: set_total from `fetched`, tick_to bumped on every
    // iteration, flush wrote the final value. Two releases means
    // total=2/current=2 at the end of the tick.
    assert_eq!(run.progress_total, Some(2));
    assert_eq!(run.progress_current, Some(2));
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
        Arc::new(td_resolution::query_builder::QueryBuilder::with_defaults()),
        None,
        detached_events(),
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
        Arc::new(td_resolution::query_builder::QueryBuilder::with_defaults()),
        None,
        detached_events(),
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
        detached_events(),
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
    // Phase string was set before the refresh call, so the in-flight
    // pill shows "Running... (refreshing)". No total / current because
    // the trait gives no inner-phase signal yet.
    assert_eq!(refreshes[0].progress_phase.as_deref(), Some("refreshing"));
}

#[tokio::test]
async fn refresh_tick_leaves_manual_series_untouched() {
    use sea_orm::{ActiveModelTrait, Set};
    use td_db::entities::series;

    let db = fresh_db().await;

    // Operator-authored manual series: no provider mapping, metadata_source="manual".
    let created = series::ActiveModel {
        canonical_title: Set("Hand-Entered Series".into()),
        alternate_titles_json: Set(None),
        cover_url: Set(None),
        kind: Set(Some("manga".into())),
        status: Set(None),
        year: Set(Some(2021)),
        description: Set(None),
        metadata_json: Set(None),
        metadata_source: Set("manual".into()),
        metadata_hash: Set(None),
        metadata_fetched_at: Set(1_700_000_000),
        first_seen_at: Set(1_700_000_000),
        last_release_at: Set(1_700_000_000),
        highest_volume: Set(None),
        highest_chapter: Set(None),
        owned: Set(0),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();

    // A full cache refresh (the riskiest job for clobbering catalog rows).
    let provider = Arc::new(FakeProvider::new(
        "mangabaka",
        RefreshStatus::Refreshed {
            records: 999,
            version: Some("v2".into()),
        },
    ));
    let locks = Arc::new(JobLocks::default());
    jobs::refresh_provider_cache::run_tick(
        provider as Arc<dyn MetadataProvider>,
        db.clone(),
        locks,
        detached_events(),
        "cron",
    )
    .await;

    // The manual series row must be byte-for-byte what we inserted: the
    // refresh job only touches the provider dump + provider_cache_state.
    let after = series::Entity::find_by_id(created.id)
        .one(&db)
        .await
        .unwrap()
        .expect("manual series should still exist after a refresh");
    assert_eq!(
        after, created,
        "refresh must not mutate a manual series row"
    );
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
        detached_events(),
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
        detached_events(),
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
// refresh_series_metadata::run_tick
// -----------------------------------------------------------------------------

/// Insert a series row + its active-provider external_id mapping. Returns
/// the new series id. Timestamps are fully explicit so the staleness
/// query has predictable inputs.
async fn seed_stale_series(
    db: &DatabaseConnection,
    title: &str,
    fetched_at: i64,
    metadata_hash: Option<&str>,
    metadata_source: &str,
    provider: &str,
    external_id: &str,
) -> i32 {
    use sea_orm::{ActiveValue::Set, EntityTrait};
    let model = td_db::entities::series::ActiveModel {
        canonical_title: Set(title.into()),
        metadata_source: Set(metadata_source.into()),
        metadata_hash: Set(metadata_hash.map(str::to_string)),
        metadata_fetched_at: Set(fetched_at),
        first_seen_at: Set(fetched_at),
        last_release_at: Set(fetched_at),
        owned: Set(0),
        ..Default::default()
    };
    let sid = td_db::entities::series::Entity::insert(model)
        .exec_with_returning(db)
        .await
        .unwrap()
        .id;
    let mapping = td_db::entities::series_external_ids::ActiveModel {
        provider: Set(provider.into()),
        external_id: Set(external_id.into()),
        series_id: Set(sid),
        fetched_at: Set(fetched_at),
    };
    td_db::entities::series_external_ids::Entity::insert(mapping)
        .exec(db)
        .await
        .unwrap();
    sid
}

/// Build a `SeriesMetadata` with a given external_id, title, and
/// content_hash. The other fields stay at minimal defaults.
fn series_metadata(external_id: &str, title: &str, content_hash: &str) -> SeriesMetadata {
    SeriesMetadata {
        external_id: external_id.into(),
        canonical_title: title.into(),
        alternate_titles: Vec::new(),
        kind: None,
        status: None,
        year: None,
        cover_url: None,
        total_volumes: None,
        total_chapters: None,
        rating: None,
        description: None,
        genres: Vec::new(),
        tags: Vec::new(),
        foreign_ids: Vec::new(),
        raw: serde_json::json!({"id": external_id, "title": title}),
        content_hash: content_hash.into(),
    }
}

#[tokio::test]
async fn series_refresh_tick_refreshes_stale_rows_against_provider() {
    let db = fresh_db().await;
    // Two stale series mapped to mangabaka.
    let s1 = seed_stale_series(
        &db,
        "Old One",
        10,
        Some("h-old1"),
        "api",
        "mangabaka",
        "mb-1",
    )
    .await;
    let s2 = seed_stale_series(
        &db,
        "Old Two",
        20,
        Some("h-old2"),
        "api",
        "mangabaka",
        "mb-2",
    )
    .await;

    let provider = Arc::new(
        FakeProvider::new("mangabaka", RefreshStatus::NotSupported)
            .with_get(
                "mb-1",
                GetOutcome::Some(Box::new(series_metadata("mb-1", "Fresh One", "h-fresh1"))),
            )
            .with_get(
                "mb-2",
                GetOutcome::Some(Box::new(series_metadata("mb-2", "Fresh Two", "h-fresh2"))),
            ),
    );

    // now = 1_000_000_000s, min_age = 0 so everything qualifies.
    let now_ts = chrono::Utc::now().timestamp();
    assert!(now_ts > 20);

    jobs::refresh_series_metadata::run_tick(
        provider.clone() as Arc<dyn MetadataProvider>,
        db.clone(),
        Arc::new(JobLocks::default()),
        10,
        0,
        detached_events(),
        "cron",
    )
    .await;

    assert_eq!(provider.gets(), 2, "one provider.get per stale row");

    // Both series rows now carry the fresh title + hash.
    let after = td_db::entities::series::Entity::find()
        .all(&db)
        .await
        .unwrap();
    let row1 = after.iter().find(|r| r.id == s1).unwrap();
    let row2 = after.iter().find(|r| r.id == s2).unwrap();
    assert_eq!(row1.canonical_title, "Fresh One");
    assert_eq!(row1.metadata_hash.as_deref(), Some("h-fresh1"));
    assert_eq!(row2.canonical_title, "Fresh Two");
    assert_eq!(row2.metadata_hash.as_deref(), Some("h-fresh2"));

    // Exactly one series_refresh_runs row, success, with refreshed=2.
    let runs = td_db::entities::series_refresh_runs::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.status, "success");
    assert_eq!(run.considered_count, Some(2));
    assert_eq!(run.refreshed_count, Some(2));
    assert_eq!(run.unchanged_count, Some(0));
    assert_eq!(run.not_found_count, Some(0));
    assert_eq!(run.errored_count, Some(0));
    assert_eq!(run.trigger, "cron");
    assert!(run.finished_at.is_some());
    // Progress: set_total after batch select, tick_to per row, flush at
    // end. Both rows walked → total=2, current=2.
    assert_eq!(run.progress_total, Some(2));
    assert_eq!(run.progress_current, Some(2));
}

#[tokio::test]
async fn series_refresh_tick_counts_unchanged_when_hash_matches() {
    let db = fresh_db().await;
    seed_stale_series(&db, "Same", 10, Some("h-same"), "api", "mangabaka", "mb-1").await;

    // Provider returns identical content_hash so persist short-circuits.
    let provider = Arc::new(
        FakeProvider::new("mangabaka", RefreshStatus::NotSupported).with_get(
            "mb-1",
            GetOutcome::Some(Box::new(series_metadata(
                "mb-1",
                "Renamed but same payload",
                "h-same",
            ))),
        ),
    );

    jobs::refresh_series_metadata::run_tick(
        provider as Arc<dyn MetadataProvider>,
        db.clone(),
        Arc::new(JobLocks::default()),
        10,
        0,
        detached_events(),
        "manual",
    )
    .await;

    let runs = td_db::entities::series_refresh_runs::Entity::find()
        .all(&db)
        .await
        .unwrap();
    let run = &runs[0];
    assert_eq!(run.status, "success");
    assert_eq!(run.refreshed_count, Some(0));
    assert_eq!(run.unchanged_count, Some(1));
    assert_eq!(run.trigger, "manual");
}

#[tokio::test]
async fn series_refresh_tick_bumps_fetched_at_when_provider_returns_none() {
    let db = fresh_db().await;
    let sid = seed_stale_series(
        &db,
        "Gone Upstream",
        10,
        Some("h-x"),
        "api",
        "mangabaka",
        "mb-gone",
    )
    .await;

    let provider = Arc::new(
        FakeProvider::new("mangabaka", RefreshStatus::NotSupported)
            .with_get("mb-gone", GetOutcome::NotFound),
    );

    let before = chrono::Utc::now().timestamp();
    jobs::refresh_series_metadata::run_tick(
        provider as Arc<dyn MetadataProvider>,
        db.clone(),
        Arc::new(JobLocks::default()),
        10,
        0,
        detached_events(),
        "cron",
    )
    .await;

    // metadata_fetched_at advanced so the row rotates out of next batch.
    let row = td_db::entities::series::Entity::find_by_id(sid)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        row.metadata_fetched_at >= before,
        "got {} expected >= {before}",
        row.metadata_fetched_at
    );
    // Content fields are untouched.
    assert_eq!(row.canonical_title, "Gone Upstream");

    let runs = td_db::entities::series_refresh_runs::Entity::find()
        .all(&db)
        .await
        .unwrap();
    let run = &runs[0];
    assert_eq!(run.status, "success");
    assert_eq!(run.not_found_count, Some(1));
    assert_eq!(run.refreshed_count, Some(0));
}

#[tokio::test]
async fn series_refresh_tick_aborts_batch_on_provider_error() {
    let db = fresh_db().await;
    seed_stale_series(&db, "First", 10, Some("h-1"), "api", "mangabaka", "mb-fail").await;
    seed_stale_series(&db, "Second", 20, Some("h-2"), "api", "mangabaka", "mb-ok").await;

    // First call errors; the second never happens because the batch breaks.
    let provider = Arc::new(
        FakeProvider::new("mangabaka", RefreshStatus::NotSupported)
            .with_get("mb-fail", GetOutcome::Err("upstream timeout".into())),
    );

    jobs::refresh_series_metadata::run_tick(
        provider.clone() as Arc<dyn MetadataProvider>,
        db.clone(),
        Arc::new(JobLocks::default()),
        10,
        0,
        detached_events(),
        "cron",
    )
    .await;

    assert_eq!(
        provider.gets(),
        1,
        "tick should abort after the first provider failure"
    );

    let runs = td_db::entities::series_refresh_runs::Entity::find()
        .all(&db)
        .await
        .unwrap();
    let run = &runs[0];
    assert_eq!(run.status, "failure");
    assert_eq!(run.errored_count, Some(1));
    assert!(run.error_message.is_some());
}

#[tokio::test]
async fn series_refresh_tick_with_held_lock_is_a_noop() {
    let db = fresh_db().await;
    seed_stale_series(&db, "X", 10, Some("h-x"), "api", "mangabaka", "mb-x").await;

    let provider = Arc::new(
        FakeProvider::new("mangabaka", RefreshStatus::NotSupported).with_get(
            "mb-x",
            GetOutcome::Some(Box::new(series_metadata("mb-x", "Fresh", "h-fresh"))),
        ),
    );
    let locks = Arc::new(JobLocks::default());
    let lock = locks.series_refresh_lock("mangabaka");
    let _guard = lock.lock().await;

    jobs::refresh_series_metadata::run_tick(
        provider.clone() as Arc<dyn MetadataProvider>,
        db.clone(),
        locks.clone(),
        10,
        0,
        detached_events(),
        "cron",
    )
    .await;

    assert_eq!(provider.gets(), 0, "provider must not be called");
    let runs = td_db::entities::series_refresh_runs::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "skipped");
    assert_eq!(
        runs[0].error_message.as_deref(),
        Some("previous refresh still running")
    );
}

#[tokio::test]
async fn series_refresh_tick_with_empty_batch_records_success() {
    let db = fresh_db().await;
    // No seeded series rows: selection query returns empty.
    let provider = Arc::new(FakeProvider::new("mangabaka", RefreshStatus::NotSupported));

    jobs::refresh_series_metadata::run_tick(
        provider.clone() as Arc<dyn MetadataProvider>,
        db.clone(),
        Arc::new(JobLocks::default()),
        50,
        0,
        detached_events(),
        "cron",
    )
    .await;

    assert_eq!(provider.gets(), 0);
    let runs = td_db::entities::series_refresh_runs::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.status, "success");
    assert_eq!(run.considered_count, Some(0));
    assert_eq!(run.refreshed_count, Some(0));
}

#[tokio::test]
async fn series_refresh_tick_does_not_overwrite_manual_rows() {
    let db = fresh_db().await;
    // Manual row that's old enough to qualify but should be excluded by
    // the selection query. Pair with a non-manual row so the tick has
    // something to do.
    seed_stale_series(
        &db,
        "Operator-curated",
        10,
        Some("operator-stamp"),
        "manual",
        "mangabaka",
        "mb-manual",
    )
    .await;
    seed_stale_series(
        &db,
        "Normal",
        20,
        Some("h-normal"),
        "api",
        "mangabaka",
        "mb-normal",
    )
    .await;

    // Script `mb-manual` to "fresh" data; if the selection ever picks it
    // up the tick would overwrite the manual row. The scripted outcome
    // is also a tripwire: we'll assert .gets() == 1 (only mb-normal).
    let provider = Arc::new(
        FakeProvider::new("mangabaka", RefreshStatus::NotSupported)
            .with_get(
                "mb-manual",
                GetOutcome::Some(Box::new(series_metadata(
                    "mb-manual",
                    "Provider Override",
                    "h-bad",
                ))),
            )
            .with_get(
                "mb-normal",
                GetOutcome::Some(Box::new(series_metadata(
                    "mb-normal",
                    "Normal Fresh",
                    "h-normal-fresh",
                ))),
            ),
    );

    jobs::refresh_series_metadata::run_tick(
        provider.clone() as Arc<dyn MetadataProvider>,
        db.clone(),
        Arc::new(JobLocks::default()),
        10,
        0,
        detached_events(),
        "cron",
    )
    .await;

    assert_eq!(
        provider.gets(),
        1,
        "selection query should exclude manual rows"
    );

    let manual_after = td_db::entities::series::Entity::find()
        .all(&db)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.canonical_title == "Operator-curated")
        .expect("manual row should still exist");
    assert_eq!(manual_after.metadata_source, "manual");
    assert_eq!(
        manual_after.metadata_hash.as_deref(),
        Some("operator-stamp")
    );
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
            series_refresh: SeriesRefreshConfig {
                cron: Some("*/1 * * * * *".into()),
                batch_size: 50,
                min_age_days: 0,
            },
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
        query_builder: Arc::new(td_resolution::query_builder::QueryBuilder::with_defaults()),
        mangaupdates_redirector: None,
        job_events: detached_events(),
    };

    let mut scheduler = Scheduler::build(&cfg, ctx).await.unwrap();
    // 1 source job + 1 provider refresh job + 1 series refresh job + 1 review-queue snapshot job.
    assert_eq!(scheduler.job_count(), 4);
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
        query_builder: Arc::new(td_resolution::query_builder::QueryBuilder::with_defaults()),
        mangaupdates_redirector: None,
        job_events: detached_events(),
    };
    let scheduler = Scheduler::build(&cfg, ctx).await.unwrap();
    // Only the unconditional review-queue snapshot job lands; source +
    // provider jobs are skipped.
    assert_eq!(scheduler.job_count(), 1);
}

/// Series-refresh registration honours the cron toggle: present + active
/// provider registered → +1 job; absent or unmapped active provider → 0
/// extra jobs.
#[tokio::test]
async fn scheduler_registers_series_refresh_job_when_cron_set() {
    let db = fresh_db().await;
    let source = Arc::new(FakeSource::new("trusted", "fake", PollOutcome::default()));
    let mut sources_builder = SourceRegistry::builder();
    sources_builder
        .register(source.clone() as Arc<dyn DiscoverySource>)
        .unwrap();
    let sources = Arc::new(sources_builder.build());

    let provider = Arc::new(FakeProvider::new("mangabaka", RefreshStatus::NotSupported));
    let metadata = build_registry(provider);

    let cfg = AppConfig {
        metadata: MetadataConfig {
            active_provider: "mangabaka".into(),
            series_refresh: SeriesRefreshConfig {
                cron: Some("*/30 * * * *".into()),
                batch_size: 25,
                min_age_days: 7,
            },
        },
        providers: ProvidersConfig {
            mangabaka: MangabakaProviderConfig {
                offline_refresh_cron: None,
                ..MangabakaProviderConfig::default()
            },
        },
        sources: vec![],
        ..Default::default()
    };

    let ctx = SchedulerContext {
        db: db.clone(),
        sources,
        metadata,
        ingestion: IngestionConfig::default(),
        locks: Arc::new(JobLocks::default()),
        query_builder: Arc::new(td_resolution::query_builder::QueryBuilder::with_defaults()),
        mangaupdates_redirector: None,
        job_events: detached_events(),
    };

    let scheduler = Scheduler::build(&cfg, ctx).await.unwrap();
    // series-refresh job + the unconditional review-queue snapshot.
    assert_eq!(scheduler.job_count(), 2);
}

/// If the active provider isn't registered, the series-refresh job is
/// skipped with a warning rather than failing boot.
#[tokio::test]
async fn scheduler_skips_series_refresh_when_active_provider_unregistered() {
    let db = fresh_db().await;
    let sources = Arc::new(SourceRegistry::builder().build());

    // Register "mangabaka" but make the active provider point to a
    // different (unregistered) id.
    let provider = Arc::new(FakeProvider::new("mangabaka", RefreshStatus::NotSupported));
    let metadata = build_registry(provider);

    let cfg = AppConfig {
        metadata: MetadataConfig {
            active_provider: "anilist".into(),
            series_refresh: SeriesRefreshConfig {
                cron: Some("*/30 * * * *".into()),
                batch_size: 10,
                min_age_days: 7,
            },
        },
        providers: ProvidersConfig {
            mangabaka: MangabakaProviderConfig {
                offline_refresh_cron: None,
                ..MangabakaProviderConfig::default()
            },
        },
        sources: vec![],
        ..Default::default()
    };

    let ctx = SchedulerContext {
        db: db.clone(),
        sources,
        metadata,
        ingestion: IngestionConfig::default(),
        locks: Arc::new(JobLocks::default()),
        query_builder: Arc::new(td_resolution::query_builder::QueryBuilder::with_defaults()),
        mangaupdates_redirector: None,
        job_events: detached_events(),
    };

    let scheduler = Scheduler::build(&cfg, ctx).await.unwrap();
    // Only the unconditional review-queue snapshot survives.
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
        search_queries: Set(None),
        cleanup_rules_applied: Set(None),
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
        search_queries: Set(None),
        cleanup_rules_applied: Set(None),
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
