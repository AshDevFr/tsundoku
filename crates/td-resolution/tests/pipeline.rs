//! End-to-end resolution pipeline tests.
//!
//! Each test drives the resolver against a synthetic [`FakeProvider`] that
//! returns canned responses. The DB is in-memory SQLite migrated via the
//! real `migration` crate, so the assertions exercise the same schema the
//! production code runs against.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use migration::{Migrator, MigratorTrait};
use sea_orm::{
    ColumnTrait, ConnectionTrait, Database, DatabaseConnection, EntityTrait, QueryFilter, Set,
};
use td_config::{FormatTypeRule, IngestionConfig};
use td_db::entities::{release_formats, releases, review_candidates, series, series_external_ids};
use td_metadata::{
    ForeignId, MetadataProvider, MetadataRegistry, MetadataResult, SearchHit, SeriesKind,
    SeriesMetadata,
};
use td_resolution::{ResolutionPath, Resolver};

/// In-memory provider mock. Tests register canned responses by ID and
/// foreign-ID pair; the resolver's calls land here.
#[derive(Default)]
struct FakeProvider {
    id: &'static str,
    /// `external_id` -> metadata
    get_table: Mutex<HashMap<String, SeriesMetadata>>,
    /// `(foreign_provider, foreign_id)` -> metadata
    foreign_table: Mutex<HashMap<(String, String), SeriesMetadata>>,
    /// `query` -> hits, returned by `search`.
    search_table: Mutex<HashMap<String, Vec<SearchHit>>>,
    /// Records every call so we can assert on order.
    calls: Mutex<Vec<String>>,
}

impl FakeProvider {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            ..Default::default()
        }
    }

    fn register_get(&self, metadata: SeriesMetadata) {
        self.get_table
            .lock()
            .unwrap()
            .insert(metadata.external_id.clone(), metadata);
    }

    fn register_foreign(&self, foreign_provider: &str, foreign_id: &str, metadata: SeriesMetadata) {
        // Also register get_table so subsequent `get()` calls succeed.
        self.get_table
            .lock()
            .unwrap()
            .insert(metadata.external_id.clone(), metadata.clone());
        self.foreign_table
            .lock()
            .unwrap()
            .insert((foreign_provider.into(), foreign_id.into()), metadata);
    }

    fn register_search(&self, query: &str, hits: Vec<SearchHit>) {
        self.search_table.lock().unwrap().insert(query.into(), hits);
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl MetadataProvider for FakeProvider {
    fn id(&self) -> &str {
        self.id
    }

    fn display_name(&self) -> &str {
        "Fake"
    }

    async fn get(&self, external_id: &str) -> MetadataResult<Option<SeriesMetadata>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("get({external_id})"));
        Ok(self.get_table.lock().unwrap().get(external_id).cloned())
    }

    async fn search(&self, query: &str, _limit: u32) -> MetadataResult<Vec<SearchHit>> {
        self.calls.lock().unwrap().push(format!("search({query})"));
        Ok(self
            .search_table
            .lock()
            .unwrap()
            .get(query)
            .cloned()
            .unwrap_or_default())
    }

    async fn resolve_by_foreign_id(
        &self,
        foreign_provider: &str,
        foreign_id: &str,
    ) -> MetadataResult<Option<SeriesMetadata>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("foreign({foreign_provider},{foreign_id})"));
        Ok(self
            .foreign_table
            .lock()
            .unwrap()
            .get(&(foreign_provider.into(), foreign_id.into()))
            .cloned())
    }
}

/// Fresh in-memory SQLite + applied migrations.
async fn fresh_db() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

/// Insert a release with the given ID and external links JSON.
async fn insert_release(
    db: &DatabaseConnection,
    id: &str,
    title: &str,
    extracted_links_json: Option<&str>,
    formats: &[&str],
) {
    let row = releases::ActiveModel {
        id: Set(id.into()),
        source_kind: Set("nyaa".into()),
        source_name: Set("test".into()),
        external_id: Set(id.into()),
        title: Set(title.into()),
        link: Set(format!("https://nyaa.si/view/{id}")),
        magnet: Set(None),
        torrent_url: Set(None),
        ddl_url: Set(None),
        info_hash: Set(None),
        size_bytes: Set(None),
        files_json: Set(None),
        description_html: Set(None),
        extracted_links_json: Set(extracted_links_json.map(str::to_string)),
        posted_at: Set(1_700_000_000),
        observed_at: Set(1_700_000_100),
        series_id: Set(None),
        resolution_path: Set(None),
        resolution_confidence: Set(None),
        resolution_status: Set("unresolved".into()),
        resolution_attempts: Set(0),
        last_resolve_attempt_at: Set(None),
        volume_span_json: Set(None),
        chapter_span_json: Set(None),
        resolved_at: Set(None),
        search_queries: Set(None),
        cleanup_rules_applied: Set(None),
    };
    releases::Entity::insert(row).exec(db).await.unwrap();
    for f in formats {
        let row = release_formats::ActiveModel {
            release_id: Set(id.into()),
            format: Set((*f).into()),
        };
        release_formats::Entity::insert(row).exec(db).await.unwrap();
    }
}

fn sample_metadata() -> SeriesMetadata {
    SeriesMetadata {
        external_id: "12345".into(),
        canonical_title: "Chainsaw Man".into(),
        alternate_titles: vec!["チェンソーマン".into()],
        kind: Some(SeriesKind::Manga),
        status: None,
        year: Some(2018),
        cover_url: None,
        description: None,
        genres: vec![],
        tags: vec![],
        foreign_ids: vec![
            ForeignId {
                provider: "mangaupdates".into(),
                id: "ylx5wzn".into(),
                url: None,
            },
            ForeignId {
                provider: "anilist".into(),
                id: "105778".into(),
                url: None,
            },
        ],
        raw: serde_json::json!({"id": 12345}),
        content_hash: "hash-12345".into(),
    }
}

fn novel_metadata() -> SeriesMetadata {
    SeriesMetadata {
        external_id: "99999".into(),
        canonical_title: "Some Light Novel".into(),
        alternate_titles: vec![],
        kind: Some(SeriesKind::Novel),
        status: None,
        year: None,
        cover_url: None,
        description: None,
        genres: vec![],
        tags: vec![],
        foreign_ids: vec![],
        raw: serde_json::json!({"id": 99999}),
        content_hash: "hash-99999".into(),
    }
}

fn build_registry(provider: Arc<FakeProvider>) -> Arc<MetadataRegistry> {
    let mut b = MetadataRegistry::builder();
    b.register(provider).unwrap();
    b.set_active("mb");
    Arc::new(b.build().unwrap())
}

fn make_resolver(
    db: &DatabaseConnection,
    registry: Arc<MetadataRegistry>,
    cfg: IngestionConfig,
) -> Resolver {
    Resolver::new(db.clone(), registry, cfg)
}

fn ingestion_default() -> IngestionConfig {
    IngestionConfig::default()
}

fn ingestion_with_rules(rules: Vec<FormatTypeRule>) -> IngestionConfig {
    IngestionConfig {
        format_type_rules: rules,
        ..IngestionConfig::default()
    }
}

#[tokio::test]
async fn known_external_id_short_circuits_with_confidence_one() {
    let db = fresh_db().await;
    // Pre-seed: a series row + (mangaupdates, ylx5wzn) mapping.
    db.execute_unprepared(
        "INSERT INTO series (id, canonical_title, metadata_source, metadata_fetched_at,\
         first_seen_at, last_release_at, owned) VALUES (1, 'Chainsaw Man', 'api', 0, 0, 0, 0)",
    )
    .await
    .unwrap();
    db.execute_unprepared(
        "INSERT INTO series_external_ids (provider, external_id, series_id, fetched_at) \
         VALUES ('mangaupdates', 'ylx5wzn', 1, 0)",
    )
    .await
    .unwrap();

    let links = serde_json::json!({
        "mangaupdates": "https://www.mangaupdates.com/series/ylx5wzn/chainsaw-man",
        "anilist": null,
        "mal": null,
        "mangadex": null,
    });
    insert_release(
        &db,
        "r1",
        "Chainsaw Man v1",
        Some(&links.to_string()),
        &["cbz"],
    )
    .await;

    let provider = Arc::new(FakeProvider::new("mb"));
    let registry = build_registry(provider.clone());
    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r1").await.unwrap();

    assert_eq!(out.path, Some(ResolutionPath::KnownExternalId));
    assert_eq!(out.series_id, Some(1));
    assert_eq!(out.confidence, Some(1.0));
    // No provider calls: step 1 short-circuited.
    assert!(provider.calls().is_empty(), "got {:?}", provider.calls());

    let stored = releases::Entity::find_by_id("r1".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.resolution_status, "resolved");
    assert_eq!(stored.series_id, Some(1));
}

#[tokio::test]
async fn foreign_id_lookup_persists_both_mangabaka_and_mangaupdates_external_ids() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mb"));
    provider.register_foreign("mangaupdates", "ylx5wzn", sample_metadata());
    let registry = build_registry(provider.clone());

    let links = serde_json::json!({
        "mangaupdates": "https://www.mangaupdates.com/series/ylx5wzn/chainsaw-man",
        "anilist": null,
        "mal": null,
        "mangadex": null,
    });
    insert_release(
        &db,
        "r2",
        "Random Title",
        Some(&links.to_string()),
        &["cbz"],
    )
    .await;

    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r2").await.unwrap();

    assert_eq!(out.path, Some(ResolutionPath::ForeignIdLookup));
    assert_eq!(out.confidence, Some(1.0));
    assert_eq!(provider.calls(), vec!["foreign(mangaupdates,ylx5wzn)"]);

    let stored = releases::Entity::find_by_id("r2".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.resolution_status, "resolved");
    let series_id = stored.series_id.unwrap();

    // The active provider's own id ("mb") plus every foreign id MangaBaka
    // surfaced lands in series_external_ids.
    let map = series_external_ids::Entity::find()
        .filter(series_external_ids::Column::SeriesId.eq(series_id))
        .all(&db)
        .await
        .unwrap();
    let mut providers: Vec<&str> = map.iter().map(|m| m.provider.as_str()).collect();
    providers.sort();
    assert_eq!(providers, vec!["anilist", "mangaupdates", "mb"]);
}

#[tokio::test]
async fn second_resolve_is_idempotent_no_duplicate_external_ids() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mb"));
    provider.register_foreign("mangaupdates", "ylx5wzn", sample_metadata());
    let registry = build_registry(provider.clone());

    let links = serde_json::json!({
        "mangaupdates": "https://www.mangaupdates.com/series/ylx5wzn/chainsaw-man",
        "anilist": null,
        "mal": null,
        "mangadex": null,
    });
    insert_release(
        &db,
        "r3",
        "Random Title",
        Some(&links.to_string()),
        &["cbz"],
    )
    .await;

    let resolver = make_resolver(&db, registry, ingestion_default());
    let first = resolver.resolve_one("r3").await.unwrap();
    let second = resolver.resolve_one("r3").await.unwrap();
    assert_eq!(first.series_id, second.series_id);

    // Series row count is 1.
    let series_count = series::Entity::find().all(&db).await.unwrap().len();
    assert_eq!(series_count, 1);

    // series_external_ids count = mb + mangaupdates + anilist
    let map_count = series_external_ids::Entity::find()
        .all(&db)
        .await
        .unwrap()
        .len();
    assert_eq!(map_count, 3);

    // resolution_attempts incremented on each run.
    let stored = releases::Entity::find_by_id("r3".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.resolution_attempts, 2);

    // Second pass hits the known-external-id step, so no foreign-id call.
    let calls = provider.calls();
    assert_eq!(
        calls.iter().filter(|c| c.starts_with("foreign")).count(),
        1,
        "got calls: {calls:?}"
    );
}

#[tokio::test]
async fn fuzzy_title_above_threshold_resolves() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mb"));
    provider.register_get(sample_metadata());
    // Very close: release title is "Chainsaw Man" with stray punctuation,
    // provider's hit is the exact canonical title.
    provider.register_search(
        "Chainsaw Man!",
        vec![SearchHit {
            external_id: "12345".into(),
            title: "Chainsaw Man".into(),
            year: None,
            cover_url: None,
            score: Some(0.95),
        }],
    );
    let registry = build_registry(provider.clone());

    insert_release(&db, "r4", "Chainsaw Man!", None, &["cbz"]).await;

    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r4").await.unwrap();
    assert_eq!(out.path, Some(ResolutionPath::FuzzyTitle));
    assert!(out.confidence.unwrap() >= 0.85);

    let stored = releases::Entity::find_by_id("r4".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.resolution_status, "resolved");
}

#[tokio::test]
async fn fuzzy_title_resolves_noisy_raw_title_via_cleaned_query() {
    // Regression test for the original Solo Leveling failure: the raw
    // nyaa title carries `(2021-2026) (Digital) (1r0n)` noise that, when
    // Diced directly against the MangaBaka candidate, drops the score
    // below the 0.85 threshold. After Phase B the cleaner produces
    // "Solo Leveling" and Dice against THAT yields 1.0.
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mb"));
    let metadata = SeriesMetadata {
        external_id: "77777".into(),
        canonical_title: "Solo Leveling".into(),
        alternate_titles: vec![],
        kind: Some(SeriesKind::Manhwa),
        status: None,
        year: Some(2018),
        cover_url: None,
        description: None,
        genres: vec![],
        tags: vec![],
        foreign_ids: vec![],
        raw: serde_json::json!({"id": 77777}),
        content_hash: "hash-77777".into(),
    };
    provider.register_get(metadata.clone());
    // Provider answers searches keyed on the CLEANED query, not the raw
    // title — that's what the resolver should send.
    provider.register_search(
        "Solo Leveling",
        vec![SearchHit {
            external_id: "77777".into(),
            title: "Solo Leveling".into(),
            year: Some(2018),
            cover_url: None,
            score: Some(0.99),
        }],
    );
    let registry = build_registry(provider.clone());

    insert_release(
        &db,
        "r-solo-fuzzy",
        "Solo Leveling (2021-2026) (Digital) (1r0n)",
        None,
        &["cbz"],
    )
    .await;

    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r-solo-fuzzy").await.unwrap();
    assert_eq!(out.path, Some(ResolutionPath::FuzzyTitle));
    assert_eq!(
        out.status,
        td_resolution::ResolutionStatus::Resolved,
        "expected auto-resolve after cleanup; got {out:?}"
    );
    let confidence = out.confidence.expect("confidence must be set");
    assert!(
        confidence >= 0.85,
        "confidence should clear threshold once Dice runs against cleaned query; got {confidence}"
    );

    // Cleaned queries persisted alongside the resolved release.
    let stored = releases::Entity::find_by_id("r-solo-fuzzy".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.resolution_status, "resolved");
    let queries: Vec<String> =
        serde_json::from_str(stored.search_queries.as_deref().unwrap_or("[]")).unwrap();
    assert_eq!(queries, vec!["Solo Leveling".to_string()]);
    let rules: Vec<String> =
        serde_json::from_str(stored.cleanup_rules_applied.as_deref().unwrap_or("[]")).unwrap();
    assert!(rules.contains(&"strip_parens".to_string()));
}

#[tokio::test]
async fn fuzzy_title_with_multi_title_separator_searches_all_halves() {
    // Multi-title separator case: raw release has English | romaji.
    // Provider only knows the romaji half; the cleaner emits both, the
    // resolver searches each, and the romaji match wins.
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mb"));
    let metadata = SeriesMetadata {
        external_id: "88888".into(),
        canonical_title: "Kamitachi ni Hirowareta Otoko".into(),
        alternate_titles: vec!["By the Grace of the Gods".into()],
        kind: Some(SeriesKind::Manga),
        status: None,
        year: None,
        cover_url: None,
        description: None,
        genres: vec![],
        tags: vec![],
        foreign_ids: vec![],
        raw: serde_json::json!({"id": 88888}),
        content_hash: "hash-88888".into(),
    };
    provider.register_get(metadata.clone());
    provider.register_search(
        "Kamitachi ni Hirowareta Otoko",
        vec![SearchHit {
            external_id: "88888".into(),
            title: "Kamitachi ni Hirowareta Otoko".into(),
            year: None,
            cover_url: None,
            score: Some(0.99),
        }],
    );
    // No registration for the English half — provider returns empty.

    let registry = build_registry(provider.clone());
    insert_release(
        &db,
        "r-multi",
        "By the Grace of the Gods | Kamitachi ni Hirowareta Otoko",
        None,
        &["cbz"],
    )
    .await;

    let resolver = make_resolver(&db, registry.clone(), ingestion_default());
    let out = resolver.resolve_one("r-multi").await.unwrap();
    assert_eq!(out.path, Some(ResolutionPath::FuzzyTitle));
    assert_eq!(out.status, td_resolution::ResolutionStatus::Resolved);

    // The provider saw both queries: the resolver searched each half.
    let calls = provider.calls();
    assert!(
        calls.iter().any(|c| c.contains("Kamitachi")),
        "expected romaji query; got {calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.contains("By the Grace")),
        "expected English query; got {calls:?}"
    );

    let stored = releases::Entity::find_by_id("r-multi".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let queries: Vec<String> =
        serde_json::from_str(stored.search_queries.as_deref().unwrap_or("[]")).unwrap();
    assert_eq!(queries.len(), 2);
    let rules: Vec<String> =
        serde_json::from_str(stored.cleanup_rules_applied.as_deref().unwrap_or("[]")).unwrap();
    assert!(rules.contains(&"split_alternates".to_string()));
}

#[tokio::test]
async fn fuzzy_title_below_threshold_queues_review_candidates() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mb"));
    // Title overlap is moderate: enough to clear review_threshold (0.55)
    // but below resolution_threshold (0.85).
    provider.register_get(sample_metadata());
    provider.register_search(
        "Chainsaw",
        vec![SearchHit {
            external_id: "12345".into(),
            title: "Chainsaw Man".into(),
            year: None,
            cover_url: None,
            score: None,
        }],
    );
    let registry = build_registry(provider.clone());
    insert_release(&db, "r5", "Chainsaw", None, &["cbz"]).await;

    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r5").await.unwrap();
    assert_eq!(out.path, None);
    assert_eq!(out.series_id, None);
    let stored = releases::Entity::find_by_id("r5".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.resolution_status, "review_pending");
    let candidates = review_candidates::Entity::find()
        .filter(review_candidates::Column::ReleaseId.eq("r5"))
        .all(&db)
        .await
        .unwrap();
    assert!(!candidates.is_empty());
}

#[tokio::test]
async fn truly_unresolved_when_search_returns_no_hits() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mb"));
    let registry = build_registry(provider.clone());
    insert_release(&db, "r6", "Some Random Title", None, &["cbz"]).await;

    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r6").await.unwrap();
    assert_eq!(out.series_id, None);
    assert_eq!(out.path, None);

    let stored = releases::Entity::find_by_id("r6".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.resolution_status, "unresolved");
}

#[tokio::test]
async fn format_type_mismatch_demotes_to_ambiguous_even_with_confident_match() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mb"));
    // Provider returns a novel-kind series, but the release contains a CBZ.
    provider.register_foreign("anilist", "1", novel_metadata());
    let registry = build_registry(provider.clone());

    let links = serde_json::json!({
        "mangaupdates": null,
        "anilist": "https://anilist.co/manga/1/Foo",
        "mal": null,
        "mangadex": null,
    });
    insert_release(
        &db,
        "r7",
        "Foo Light Novel scanlated as CBZ",
        Some(&links.to_string()),
        &["cbz"],
    )
    .await;

    let rules = vec![FormatTypeRule {
        formats: vec!["cbz".into()],
        required_kinds: vec!["manga".into(), "manhwa".into(), "manhua".into()],
    }];
    let resolver = make_resolver(&db, registry, ingestion_with_rules(rules));
    let out = resolver.resolve_one("r7").await.unwrap();
    assert_eq!(out.path, Some(ResolutionPath::ForeignIdLookup));
    let stored = releases::Entity::find_by_id("r7".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.resolution_status, "ambiguous");
    // Series link still established; review queue surfaces the mismatch
    // via the outcome.reason.
    assert!(
        out.reason
            .as_deref()
            .unwrap()
            .contains("format_type_mismatch")
    );
}

#[tokio::test]
async fn swapping_active_provider_changes_who_drives_resolution() {
    let db = fresh_db().await;
    // Two providers: only the active one's responses count.
    let a = Arc::new(FakeProvider::new("a"));
    let b = Arc::new(FakeProvider::new("b"));
    a.register_foreign(
        "mangaupdates",
        "x",
        SeriesMetadata {
            external_id: "from-a".into(),
            content_hash: "h-a".into(),
            ..sample_metadata()
        },
    );
    b.register_foreign(
        "mangaupdates",
        "x",
        SeriesMetadata {
            external_id: "from-b".into(),
            content_hash: "h-b".into(),
            ..sample_metadata()
        },
    );

    // Build registry with B active.
    let mut builder = MetadataRegistry::builder();
    builder.register(a.clone()).unwrap();
    builder.register(b.clone()).unwrap();
    builder.set_active("b");
    let registry = Arc::new(builder.build().unwrap());

    let links = serde_json::json!({
        "mangaupdates": "https://www.mangaupdates.com/series/x/foo",
        "anilist": null,
        "mal": null,
        "mangadex": null,
    });
    insert_release(&db, "r8", "Title", Some(&links.to_string()), &["cbz"]).await;
    let resolver = make_resolver(&db, registry, ingestion_default());
    let _out = resolver.resolve_one("r8").await.unwrap();

    // Only B's resolve_by_foreign_id was called.
    assert_eq!(b.calls(), vec!["foreign(mangaupdates,x)"]);
    assert!(a.calls().is_empty(), "got {:?}", a.calls());

    // The series row's external_id under 'b' is "from-b".
    let row = series_external_ids::Entity::find()
        .filter(series_external_ids::Column::Provider.eq("b"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.external_id, "from-b");
}

#[tokio::test]
async fn resolve_unresolved_picks_up_only_unresolved_and_ambiguous() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mb"));
    provider.register_foreign("mangaupdates", "ylx5wzn", sample_metadata());
    let registry = build_registry(provider.clone());

    // Three releases: one unresolved with a known foreign id, one
    // resolved already, one ambiguous.
    let links = serde_json::json!({
        "mangaupdates": "https://www.mangaupdates.com/series/ylx5wzn/x",
        "anilist": null,
        "mal": null,
        "mangadex": null,
    });
    insert_release(&db, "u1", "u1", Some(&links.to_string()), &["cbz"]).await;
    insert_release(&db, "u2", "u2", None, &["cbz"]).await;
    insert_release(&db, "u3", "u3", Some(&links.to_string()), &["cbz"]).await;
    // Mark u2 resolved so the batch should skip it.
    let model = releases::ActiveModel {
        id: Set("u2".into()),
        resolution_status: Set("resolved".into()),
        ..Default::default()
    };
    releases::Entity::update(model).exec(&db).await.unwrap();
    // Mark u3 ambiguous so the batch should pick it up too.
    let model = releases::ActiveModel {
        id: Set("u3".into()),
        resolution_status: Set("ambiguous".into()),
        ..Default::default()
    };
    releases::Entity::update(model).exec(&db).await.unwrap();

    let resolver = make_resolver(&db, registry, ingestion_default());
    let summary = resolver.resolve_unresolved(100).await.unwrap();
    assert_eq!(summary.resolved, 2, "expected u1 and u3 to resolve");
    assert_eq!(summary.errors, 0);

    let u2 = releases::Entity::find_by_id("u2".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    // Untouched by the batch — attempts still 0.
    assert_eq!(u2.resolution_attempts, 0);
    assert_eq!(u2.resolution_status, "resolved");
}

#[tokio::test]
async fn resolve_ids_targets_only_the_given_releases() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mb"));
    provider.register_foreign("mangaupdates", "ylx5wzn", sample_metadata());
    let registry = build_registry(provider.clone());

    let links = serde_json::json!({
        "mangaupdates": "https://www.mangaupdates.com/series/ylx5wzn/x",
        "anilist": null,
        "mal": null,
        "mangadex": null,
    });
    insert_release(&db, "u1", "u1", Some(&links.to_string()), &["cbz"]).await;
    insert_release(&db, "u2", "u2", Some(&links.to_string()), &["cbz"]).await;

    let resolver = make_resolver(&db, registry, ingestion_default());
    // Only u1 is in the target set; u2 must be left untouched.
    let summary = resolver.resolve_ids(&["u1".to_string()]).await.unwrap();
    assert_eq!(summary.resolved, 1);
    assert_eq!(summary.errors, 0);

    let u2 = releases::Entity::find_by_id("u2".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(u2.resolution_attempts, 0, "u2 not in the target set");
    assert_eq!(u2.resolution_status, "unresolved");

    // A missing id is counted as an error, not a panic.
    let summary = resolver
        .resolve_ids(&["does-not-exist".to_string()])
        .await
        .unwrap();
    assert_eq!(summary.errors, 1);
    assert_eq!(summary.total(), 1);
}

// ---------------------------------------------------------------------------
// Legacy MangaUpdates URL → modern slug normalization
// ---------------------------------------------------------------------------

mod legacy_mu {
    //! Drives a local TCP listener that returns canned HTTP responses, so
    //! the resolver can exercise the full legacy-id translation path:
    //! `series.html?id=NNN` → cache miss → HEAD → 308 → modern slug →
    //! `resolve_by_foreign_id("mangaupdates", modern)`.

    use super::*;
    use reqwest::{Client, redirect};
    use std::io::{Read, Write};
    use std::net::TcpListener as StdListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use td_db::repos::mangaupdates_id_repo;
    use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;

    fn spawn_canned_server(responses: Vec<&'static [u8]>) -> (String, thread::JoinHandle<()>) {
        let listener = StdListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}");
        let counter = Arc::new(AtomicUsize::new(0));
        let handle = thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let idx = counter.fetch_add(1, Ordering::SeqCst);
                if idx >= responses.len() {
                    return;
                }
                let _ = stream.write_all(responses[idx]);
                let _ = stream.flush();
            }
        });
        (url, handle)
    }

    fn build_redirector(base_url: String) -> Arc<MangaUpdatesRedirector> {
        let inner = Client::builder()
            .redirect(redirect::Policy::none())
            .build()
            .unwrap();
        let client = td_http::HttpLimiter::no_limit().client(inner);
        Arc::new(MangaUpdatesRedirector::with_client(client, base_url))
    }

    fn solo_leveling_metadata() -> SeriesMetadata {
        SeriesMetadata {
            external_id: "55555".into(),
            canonical_title: "Solo Leveling".into(),
            alternate_titles: vec!["나 혼자만 레벨업".into()],
            kind: Some(SeriesKind::Manhwa),
            status: None,
            year: Some(2018),
            cover_url: None,
            description: None,
            genres: vec![],
            tags: vec![],
            foreign_ids: vec![ForeignId {
                provider: "mangaupdates".into(),
                id: "6z1uqw7".into(),
                url: None,
            }],
            raw: serde_json::json!({"id": 55555}),
            content_hash: "hash-55555".into(),
        }
    }

    #[tokio::test]
    async fn legacy_mu_url_resolves_to_modern_slug_and_persists_cache() {
        let resp =
            b"HTTP/1.1 308 Permanent Redirect\r\nContent-Length: 0\r\nLocation: https://www.mangaupdates.com/series/6z1uqw7/solo-leveling\r\n\r\n";
        let (server_url, _h) = spawn_canned_server(vec![resp]);

        let db = fresh_db().await;
        let provider = Arc::new(FakeProvider::new("mb"));
        provider.register_foreign("mangaupdates", "6z1uqw7", solo_leveling_metadata());
        let registry = build_registry(provider.clone());

        let links = serde_json::json!({
            "mangaupdates": "https://www.mangaupdates.com/series.html?id=151349",
            "anilist": null,
            "mal": null,
            "mangadex": null,
        });
        insert_release(
            &db,
            "r-solo",
            "Solo Leveling (2021-2026) (Digital) (1r0n)",
            Some(&links.to_string()),
            &["cbz"],
        )
        .await;

        let redirector = build_redirector(server_url);
        let resolver = make_resolver(&db, registry, ingestion_default())
            .with_mangaupdates_redirector(redirector);

        let outcome = resolver.resolve_one("r-solo").await.unwrap();

        // The translation step should turn the legacy URL into a modern
        // mangaupdates foreign-id lookup, which the FakeProvider answers
        // immediately. No fuzzy search runs.
        assert_eq!(outcome.path, Some(ResolutionPath::ForeignIdLookup));
        assert_eq!(outcome.status, td_resolution::ResolutionStatus::Resolved);
        let calls = provider.calls();
        assert!(
            calls.iter().any(|c| c == "foreign(mangaupdates,6z1uqw7)"),
            "expected foreign-id lookup on modern slug, got {calls:?}"
        );
        assert!(
            !calls.iter().any(|c| c.starts_with("search(")),
            "fuzzy search should not run; got {calls:?}"
        );

        // The cache row is persisted: next resolve doesn't need network.
        let cached = mangaupdates_id_repo::lookup(&db, 151349).await.unwrap();
        assert_eq!(cached, Some(Some("6z1uqw7".to_string())));
    }

    #[tokio::test]
    async fn legacy_mu_tombstone_drops_link_and_falls_through() {
        let resp =
            b"HTTP/1.1 307 Temporary Redirect\r\nContent-Length: 0\r\nLocation: /series\r\n\r\n";
        let (server_url, _h) = spawn_canned_server(vec![resp]);

        let db = fresh_db().await;
        let provider = Arc::new(FakeProvider::new("mb"));
        // No foreign mapping registered; nothing to resolve to.
        let registry = build_registry(provider.clone());

        let links = serde_json::json!({
            "mangaupdates": "https://www.mangaupdates.com/series.html?id=99999999",
            "anilist": null,
            "mal": null,
            "mangadex": null,
        });
        insert_release(
            &db,
            "r-dead",
            "Dead Series v01",
            Some(&links.to_string()),
            &[],
        )
        .await;

        let redirector = build_redirector(server_url);
        let resolver = make_resolver(&db, registry, ingestion_default())
            .with_mangaupdates_redirector(redirector);

        let outcome = resolver.resolve_one("r-dead").await.unwrap();

        // No external mapping, no candidates → unresolved.
        assert_eq!(outcome.path, None);
        assert_eq!(outcome.status, td_resolution::ResolutionStatus::Unresolved);
        // foreign-id was never called: the tombstone dropped the link
        // before step 2 could see it.
        let calls = provider.calls();
        assert!(
            !calls.iter().any(|c| c.starts_with("foreign(")),
            "tombstoned link should not reach foreign-id lookup; got {calls:?}"
        );

        // Tombstone is persisted for next-poll fast-path.
        let cached = mangaupdates_id_repo::lookup(&db, 99_999_999).await.unwrap();
        assert_eq!(cached, Some(None));
    }

    #[tokio::test]
    async fn legacy_mu_cache_hit_skips_network() {
        // Pre-seed the cache; no listener responses configured, so any
        // network call would fail or hang.
        let db = fresh_db().await;
        mangaupdates_id_repo::record(&db, 151349, Some("6z1uqw7"), 1_700_000_000)
            .await
            .unwrap();

        let provider = Arc::new(FakeProvider::new("mb"));
        provider.register_foreign("mangaupdates", "6z1uqw7", solo_leveling_metadata());
        let registry = build_registry(provider.clone());

        let links = serde_json::json!({
            "mangaupdates": "https://www.mangaupdates.com/series.html?id=151349",
            "anilist": null,
            "mal": null,
            "mangadex": null,
        });
        insert_release(
            &db,
            "r-solo2",
            "Solo Leveling (2021-2026) (Digital) (1r0n)",
            Some(&links.to_string()),
            &["cbz"],
        )
        .await;

        // Build a redirector against a port nothing listens on. The
        // cache hit must short-circuit before we try the wire.
        let dead_url = "http://127.0.0.1:1".to_string();
        let redirector = build_redirector(dead_url);
        let resolver = make_resolver(&db, registry, ingestion_default())
            .with_mangaupdates_redirector(redirector);

        let outcome = resolver.resolve_one("r-solo2").await.unwrap();
        assert_eq!(outcome.path, Some(ResolutionPath::ForeignIdLookup));
        assert_eq!(outcome.status, td_resolution::ResolutionStatus::Resolved);
    }
}
