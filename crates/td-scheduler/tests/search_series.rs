//! Integration tests for the per-series search driver
//! (`jobs::search_series::run`): scripted `SearchSource` + in-memory
//! SQLite + a metadata-provider double, exercising the walk, dedup,
//! pagination cap, and the `search_runs` audit lifecycle.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use migration::{Migrator, MigratorTrait};
use sea_orm::ActiveValue::Set;
use sea_orm::{Database, DatabaseConnection, EntityTrait};
use td_config::IngestionConfig;
use td_db::entities::series;
use td_db::repos::releases_repo::{self, id_for};
use td_db::repos::search_runs_repo;
use td_metadata::{MetadataProvider, MetadataRegistry, MetadataResult, SearchHit, SeriesMetadata};
use td_resolution::query_builder::QueryBuilder;
use td_scheduler::jobs::search_series;
use td_source::{DiscoveredRelease, SearchSource, SourceError, SourceResult};

// -----------------------------------------------------------------------------
// Doubles
// -----------------------------------------------------------------------------

/// Scripted search endpoint: maps `(query, page)` to a fixed hit list.
/// Unscripted pages return empty (ends the walk). `fail_queries` makes
/// every page of those queries error.
struct StubSearch {
    name: String,
    pages: HashMap<(String, u32), Vec<DiscoveredRelease>>,
    fail_queries: Vec<String>,
    calls: AtomicUsize,
}

impl StubSearch {
    fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            pages: HashMap::new(),
            fail_queries: Vec::new(),
            calls: AtomicUsize::new(0),
        }
    }

    fn with_page(mut self, query: &str, page: u32, hits: Vec<DiscoveredRelease>) -> Self {
        self.pages.insert((query.into(), page), hits);
        self
    }

    fn failing_on(mut self, query: &str) -> Self {
        self.fail_queries.push(query.into());
        self
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl SearchSource for StubSearch {
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> &str {
        "fake"
    }
    async fn search_page(&self, query: &str, page: u32) -> SourceResult<Vec<DiscoveredRelease>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_queries.iter().any(|q| q == query) {
            return Err(SourceError::Unavailable {
                source_kind: "fake".into(),
                source_name: self.name.clone(),
                source: anyhow::anyhow!("scripted failure"),
            });
        }
        Ok(self
            .pages
            .get(&(query.to_string(), page))
            .cloned()
            .unwrap_or_default())
    }
}

/// Inert metadata provider: never matches anything, so every persisted
/// release stays unresolved — resolution behavior is pipeline-tested
/// elsewhere; these tests only care that the resolver was driven safely.
struct InertProvider;

#[async_trait]
impl MetadataProvider for InertProvider {
    fn id(&self) -> &str {
        "inert"
    }
    fn display_name(&self) -> &str {
        "Inert"
    }
    async fn get(&self, _external_id: &str) -> MetadataResult<Option<SeriesMetadata>> {
        Ok(None)
    }
    async fn search(&self, _query: &str, _limit: u32) -> MetadataResult<Vec<SearchHit>> {
        Ok(Vec::new())
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

async fn insert_series(db: &DatabaseConnection, title: &str, alternates: &[&str]) -> i32 {
    let alt_json = if alternates.is_empty() {
        None
    } else {
        Some(serde_json::to_string(alternates).unwrap())
    };
    let model = series::ActiveModel {
        canonical_title: Set(title.into()),
        alternate_titles_json: Set(alt_json),
        metadata_source: Set("test".into()),
        metadata_fetched_at: Set(0),
        first_seen_at: Set(0),
        last_release_at: Set(0),
        owned: Set(0),
        ..Default::default()
    };
    series::Entity::insert(model)
        .exec_with_returning(db)
        .await
        .unwrap()
        .id
}

fn hit(external_id: &str, title: &str) -> DiscoveredRelease {
    DiscoveredRelease {
        source_kind: "fake".into(),
        source_name: "stub-search".into(),
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
        comment_suggested_links: Default::default(),
        information_url: None,
        posted_at: Utc::now(),
    }
}

fn registry() -> Arc<MetadataRegistry> {
    let mut b = MetadataRegistry::builder();
    b.register(Arc::new(InertProvider) as Arc<dyn MetadataProvider>)
        .unwrap();
    b.set_active("inert");
    Arc::new(b.build().unwrap())
}

async fn run(
    source: Arc<StubSearch>,
    max_pages: u32,
    db: &DatabaseConnection,
    series_id: i32,
) -> anyhow::Result<search_series::SearchSummary> {
    search_series::run(
        source,
        max_pages,
        db.clone(),
        registry(),
        IngestionConfig::default(),
        Arc::new(QueryBuilder::new(&[]).unwrap()),
        None,
        series_id,
        "cli",
    )
    .await
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[tokio::test]
async fn walks_queries_and_persists_hits_with_audit_row() {
    let db = fresh_db().await;
    let series_id = insert_series(&db, "Solo Leveling", &["Only I Level Up"]).await;

    let source = Arc::new(
        StubSearch::new("stub-search")
            .with_page("Solo Leveling", 1, vec![hit("a1", "Solo Leveling v01")])
            .with_page("Only I Level Up", 1, vec![hit("b1", "Only I Level Up v02")]),
    );
    let summary = run(source, 5, &db, series_id).await.unwrap();

    assert_eq!(summary.queries_attempted, 2);
    assert_eq!(summary.releases_seen, 2);
    assert_eq!(summary.releases_new, 2);
    assert_eq!(summary.already_known, 0);
    assert_eq!(summary.errors, 0);

    // Both hits are in the catalog.
    for ext in ["a1", "b1"] {
        assert!(
            releases_repo::find_by_id(&db, &id_for("fake", ext))
                .await
                .unwrap()
                .is_some(),
            "release {ext} should be persisted"
        );
    }

    // Audit row completed as success with matching counts.
    let runs = search_runs_repo::recent_for_series(&db, series_id, 10)
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    let row = &runs[0];
    assert_eq!(row.outcome, search_runs_repo::OUTCOME_SUCCESS);
    assert_eq!(row.search_name, "stub-search");
    assert_eq!(row.trigger, "cli");
    assert_eq!(row.queries_attempted, Some(2));
    assert_eq!(row.releases_seen, Some(2));
    assert_eq!(row.releases_new, Some(2));
    assert!(row.finished_at.is_some());
}

#[tokio::test]
async fn dedupes_the_same_hit_across_queries() {
    let db = fresh_db().await;
    let series_id = insert_series(&db, "Frieren", &["Sousou no Frieren"]).await;

    // Both title queries surface the same post.
    let source = Arc::new(
        StubSearch::new("stub-search")
            .with_page("Frieren", 1, vec![hit("same", "Frieren v01")])
            .with_page("Sousou no Frieren", 1, vec![hit("same", "Frieren v01")]),
    );
    let summary = run(source, 5, &db, series_id).await.unwrap();

    assert_eq!(summary.releases_seen, 2);
    assert_eq!(summary.releases_new, 1);
    assert_eq!(summary.already_known, 0);
}

#[tokio::test]
async fn skips_releases_already_in_the_catalog() {
    let db = fresh_db().await;
    let series_id = insert_series(&db, "Berserk", &[]).await;

    let existing = hit("known", "Berserk v01");
    releases_repo::persist_discovered(&db, &existing, 123)
        .await
        .unwrap();

    let source = Arc::new(StubSearch::new("stub-search").with_page(
        "Berserk",
        1,
        vec![hit("known", "Berserk v01"), hit("fresh", "Berserk v02")],
    ));
    let summary = run(source, 5, &db, series_id).await.unwrap();

    assert_eq!(summary.releases_seen, 2);
    assert_eq!(summary.already_known, 1);
    assert_eq!(summary.releases_new, 1);
}

#[tokio::test]
async fn caps_the_page_walk_at_max_pages() {
    let db = fresh_db().await;
    let series_id = insert_series(&db, "One Piece Box", &[]).await;

    // Every page up to 10 has content; the cap must stop the walk at 2.
    let mut source = StubSearch::new("stub-search");
    for page in 1..=10u32 {
        source = source.with_page(
            "One Piece Box",
            page,
            vec![hit(&format!("p{page}"), &format!("One Piece v{page:02}"))],
        );
    }
    let source = Arc::new(source);
    let summary = run(source.clone(), 2, &db, series_id).await.unwrap();

    assert_eq!(summary.pages_fetched, 2);
    assert_eq!(source.calls(), 2);
    assert_eq!(summary.releases_new, 2);
}

#[tokio::test]
async fn stops_a_query_walk_on_the_first_empty_page() {
    let db = fresh_db().await;
    let series_id = insert_series(&db, "Vinland Saga", &[]).await;

    // Page 1 has a hit, page 2 is unscripted (empty) — page 3 must never
    // be requested even though max_pages allows it.
    let source = Arc::new(StubSearch::new("stub-search").with_page(
        "Vinland Saga",
        1,
        vec![hit("v1", "Vinland Saga v01")],
    ));
    let summary = run(source.clone(), 5, &db, series_id).await.unwrap();

    assert_eq!(summary.pages_fetched, 2, "content page + empty page");
    assert_eq!(source.calls(), 2);
}

#[tokio::test]
async fn records_error_outcome_when_every_query_fails() {
    let db = fresh_db().await;
    let series_id = insert_series(&db, "Dead Upstream", &[]).await;

    let source = Arc::new(StubSearch::new("stub-search").failing_on("Dead Upstream"));
    let summary = run(source, 5, &db, series_id).await.unwrap();

    assert_eq!(summary.errors, 1);
    assert_eq!(summary.releases_new, 0);

    let runs = search_runs_repo::recent_for_series(&db, series_id, 10)
        .await
        .unwrap();
    assert_eq!(runs[0].outcome, search_runs_repo::OUTCOME_ERROR);
    assert!(runs[0].error.is_some());
    assert!(runs[0].finished_at.is_some());
}

#[tokio::test]
async fn one_failing_query_does_not_sink_the_run() {
    let db = fresh_db().await;
    let series_id = insert_series(&db, "Kingdom", &["킹덤"]).await;

    let source = Arc::new(
        StubSearch::new("stub-search")
            .failing_on("Kingdom")
            .with_page("킹덤", 1, vec![hit("k1", "Kingdom v01")]),
    );
    let summary = run(source, 5, &db, series_id).await.unwrap();

    assert_eq!(summary.errors, 1);
    assert_eq!(summary.releases_new, 1);
    let runs = search_runs_repo::recent_for_series(&db, series_id, 10)
        .await
        .unwrap();
    assert_eq!(runs[0].outcome, search_runs_repo::OUTCOME_SUCCESS);
}

#[tokio::test]
async fn missing_series_is_a_setup_error() {
    let db = fresh_db().await;
    let source = Arc::new(StubSearch::new("stub-search"));
    let err = run(source, 5, &db, 9999).await.unwrap_err();
    assert!(err.to_string().contains("9999"));
    // No audit row: the FK target doesn't exist.
}
