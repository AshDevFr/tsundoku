//! Integration tests for the bulk re-enrich job
//! (`jobs::reenrich_releases::run`): stub enricher + in-memory SQLite,
//! exercising the cross-origin walk, the per-kind enricher map, the
//! missing-details narrowing, the name scope, and the per-origin
//! `poll_runs` audit rows.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use migration::{Migrator, MigratorTrait};
use sea_orm::{ColumnTrait, Database, DatabaseConnection, EntityTrait, QueryFilter};
use td_db::entities::{poll_runs, releases};
use td_db::repos::releases_repo;
use td_scheduler::jobs::reenrich_releases::{self, Enricher};
use td_source::{DiscoveredRelease, ExternalLinks, SearchSource, SourceResult};
use tokio::sync::broadcast;

/// Stub upstream: `enrich` stamps a recognizable description + file list,
/// standing in for a detail-page fetch. Registered as a `SearchSource`
/// because that trait has the smallest surface; the job treats both trait
/// kinds identically.
struct StubEnricher {
    name: String,
    kind: String,
    calls: AtomicUsize,
}

impl StubEnricher {
    fn new(kind: &str) -> Arc<Self> {
        Arc::new(Self {
            name: format!("{kind}-stub"),
            kind: kind.into(),
            calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl SearchSource for StubEnricher {
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> &str {
        &self.kind
    }
    async fn search_page(&self, _query: &str, _page: u32) -> SourceResult<Vec<DiscoveredRelease>> {
        Ok(Vec::new())
    }
    async fn enrich(&self, release: &mut DiscoveredRelease) -> SourceResult<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        release.description_html = Some("enriched".into());
        release.files = vec![format!("{}.cbz", release.external_id)];
        Ok(())
    }
}

async fn fresh_db() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

fn release(kind: &str, name: &str, external_id: &str, detailed: bool) -> DiscoveredRelease {
    DiscoveredRelease {
        source_kind: kind.into(),
        source_name: name.into(),
        external_id: external_id.into(),
        title: format!("Series {external_id} v01"),
        link: format!("https://example.test/view/{external_id}"),
        magnet: None,
        torrent_url: None,
        ddl_url: None,
        info_hash: None,
        size_bytes: None,
        files: if detailed {
            vec![format!("{external_id}.cbz")]
        } else {
            Vec::new()
        },
        description_html: detailed.then(|| "already there".to_string()),
        external_links: ExternalLinks::default(),
        comment_suggested_links: ExternalLinks::default(),
        information_url: None,
        posted_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
    }
}

fn enrichers_for(stub: &Arc<StubEnricher>) -> HashMap<String, Enricher> {
    HashMap::from([(
        stub.kind.clone(),
        Enricher::Search(stub.clone() as Arc<dyn SearchSource>),
    )])
}

#[tokio::test]
async fn walks_every_origin_including_orphans_and_records_per_origin_runs() {
    let db = fresh_db().await;
    // Three origins of the same kind: a "registered" source, a search
    // entry, and an orphan whose config block no longer exists — the job
    // cannot tell them apart and must walk all three.
    releases_repo::persist_discovered(&db, &release("fake", "feed-a", "1", false), 100)
        .await
        .unwrap();
    releases_repo::persist_discovered(&db, &release("fake", "search-b", "2", false), 200)
        .await
        .unwrap();
    releases_repo::persist_discovered(&db, &release("fake", "ghost", "3", false), 300)
        .await
        .unwrap();

    let stub = StubEnricher::new("fake");
    let (events, _rx) = broadcast::channel(64);
    let summary = reenrich_releases::run(
        db.clone(),
        enrichers_for(&stub),
        vec!["unresolved".into()],
        false,
        None,
        events,
        "manual",
    )
    .await
    .unwrap();

    assert_eq!(summary.considered, 3);
    assert_eq!(summary.reenriched, 3);
    assert_eq!(summary.errors, 0);
    assert_eq!(summary.skipped_no_enricher, 0);
    assert_eq!(stub.calls.load(Ordering::SeqCst), 3);

    for id in ["fake:1", "fake:2", "fake:3"] {
        let row = releases::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.description_html.as_deref(), Some("enriched"));
        assert_eq!(row.resolution_status, "unresolved");
    }

    // One audit row per origin, on that origin's poll_runs lane.
    let runs = poll_runs::Entity::find().all(&db).await.unwrap();
    let mut names: Vec<&str> = runs.iter().map(|r| r.source_name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["feed-a", "ghost", "search-b"]);
    assert!(runs.iter().all(|r| r.status == "success"));
    assert!(runs.iter().all(|r| r.fetched_count == Some(1)));
}

#[tokio::test]
async fn only_missing_details_skips_filled_rows() {
    let db = fresh_db().await;
    releases_repo::persist_discovered(&db, &release("fake", "feed-a", "1", true), 100)
        .await
        .unwrap();
    releases_repo::persist_discovered(&db, &release("fake", "feed-a", "2", false), 200)
        .await
        .unwrap();

    let stub = StubEnricher::new("fake");
    let (events, _rx) = broadcast::channel(64);
    let summary = reenrich_releases::run(
        db.clone(),
        enrichers_for(&stub),
        vec!["unresolved".into()],
        true,
        None,
        events,
        "manual",
    )
    .await
    .unwrap();

    assert_eq!(summary.considered, 1);
    assert_eq!(summary.reenriched, 1);
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);
    let untouched = releases::Entity::find_by_id("fake:1")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(untouched.description_html.as_deref(), Some("already there"));
}

#[tokio::test]
async fn scope_limits_the_walk_and_unknown_kinds_are_counted_skipped() {
    let db = fresh_db().await;
    releases_repo::persist_discovered(&db, &release("fake", "feed-a", "1", false), 100)
        .await
        .unwrap();
    releases_repo::persist_discovered(&db, &release("fake", "feed-b", "2", false), 200)
        .await
        .unwrap();
    // A kind with no registered enricher: selected by the scope but skipped.
    releases_repo::persist_discovered(&db, &release("gone", "feed-b", "3", false), 300)
        .await
        .unwrap();

    let stub = StubEnricher::new("fake");
    let (events, _rx) = broadcast::channel(64);
    let summary = reenrich_releases::run(
        db.clone(),
        enrichers_for(&stub),
        vec!["unresolved".into()],
        false,
        Some(vec!["feed-b".into()]),
        events,
        "manual",
    )
    .await
    .unwrap();

    assert_eq!(summary.considered, 2);
    assert_eq!(summary.reenriched, 1);
    assert_eq!(summary.skipped_no_enricher, 1);
    let out_of_scope = releases::Entity::find_by_id("fake:1")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(out_of_scope.description_html.is_none());
    // Skipped-kind groups leave no audit row.
    let runs = poll_runs::Entity::find()
        .filter(poll_runs::Column::SourceKind.eq("gone"))
        .all(&db)
        .await
        .unwrap();
    assert!(runs.is_empty());
}
