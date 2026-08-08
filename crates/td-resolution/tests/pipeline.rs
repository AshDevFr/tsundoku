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
    insert_release_full(db, id, title, extracted_links_json, None, formats).await;
}

/// Like [`insert_release`] but also sets the uploader's "Information" field
/// (`information_url`) — the dedicated series pointer on a Nyaa post.
async fn insert_release_full(
    db: &DatabaseConnection,
    id: &str,
    title: &str,
    extracted_links_json: Option<&str>,
    information_url: Option<&str>,
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
        comment_suggested_links_json: Set(None),
        information_url: Set(information_url.map(str::to_string)),
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
        sent_to_client_at: Set(None),
        sent_to_client_label: Set(None),
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
        total_volumes: None,
        total_chapters: None,
        rating: None,
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
        published_start_date: None,
        published_end_date: None,
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
        total_volumes: None,
        total_chapters: None,
        rating: None,
        description: None,
        genres: vec![],
        tags: vec![],
        foreign_ids: vec![],
        raw: serde_json::json!({"id": 99999}),
        published_start_date: None,
        published_end_date: None,
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
async fn direct_active_provider_link_resolves_via_get_not_fuzzy() {
    // Regression: a Nyaa post body that embeds a `mangabaka.org/{id}` link
    // used to fall through to fuzzy matching because step 2 skipped any
    // pair whose provider matched the active provider's own id. Use a
    // provider id of `"mangabaka"` (the real value in production) so the
    // extracted link's canonical tag matches the active provider's id.
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mangabaka"));
    provider.register_get(sample_metadata());
    let mut b = MetadataRegistry::builder();
    b.register(provider.clone()).unwrap();
    b.set_active("mangabaka");
    let registry = Arc::new(b.build().unwrap());

    let links = serde_json::json!({
        "mangaupdates": null,
        "anilist": null,
        "mal": null,
        "mangadex": null,
        "mangabaka": "https://mangabaka.org/12345?utm_source=nyaa",
    });
    insert_release(
        &db,
        "r-mb-direct",
        // Garbage title on purpose: fuzzy would never match this.
        "asdf qwer zxcv 12345",
        Some(&links.to_string()),
        &["cbz"],
    )
    .await;

    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r-mb-direct").await.unwrap();

    assert_eq!(out.path, Some(ResolutionPath::ForeignIdLookup));
    assert_eq!(out.confidence, Some(1.0));
    // Provider was asked by its own id; no foreign-id call, no fuzzy search.
    let calls = provider.calls();
    assert!(
        calls.iter().any(|c| c == "get(12345)"),
        "expected get(12345); got {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.starts_with("search(")),
        "fuzzy search should not run; got {calls:?}"
    );

    let stored = releases::Entity::find_by_id("r-mb-direct".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.resolution_status, "resolved");
    // The mapping is now persisted under the active provider's id, so a
    // future release with the same link will hit step 1 instead.
    let map = series_external_ids::Entity::find()
        .filter(series_external_ids::Column::Provider.eq("mangabaka"))
        .filter(series_external_ids::Column::ExternalId.eq("12345"))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(map.len(), 1);
}

#[tokio::test]
async fn information_url_resolves_via_known_external_id_when_no_body_links() {
    // The uploader's "Information" field carries the series link while the
    // description body has none (empty extracted_links_json). A MangaUpdates
    // Information link whose id is already cataloged must resolve at step 1.
    let db = fresh_db().await;
    db.execute_unprepared(
        "INSERT INTO series (id, canonical_title, metadata_source, metadata_fetched_at,\
         first_seen_at, last_release_at, owned) VALUES (1, 'Magilumiere', 'api', 0, 0, 0, 0)",
    )
    .await
    .unwrap();
    db.execute_unprepared(
        "INSERT INTO series_external_ids (provider, external_id, series_id, fetched_at) \
         VALUES ('mangaupdates', 'd5jvvlu', 1, 0)",
    )
    .await
    .unwrap();

    insert_release_full(
        &db,
        "r-info-known",
        "Kabushiki Gaisha MagiLumiere 001-077.5",
        None, // no body links
        Some("https://www.mangaupdates.com/series/d5jvvlu/kabushiki-gaisha-magilumiere"),
        &["cbz"],
    )
    .await;

    let provider = Arc::new(FakeProvider::new("mb"));
    let registry = build_registry(provider.clone());
    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r-info-known").await.unwrap();

    assert_eq!(out.path, Some(ResolutionPath::KnownExternalId));
    assert_eq!(out.series_id, Some(1));
    assert_eq!(out.confidence, Some(1.0));
    // Step 1 short-circuited on the Information link; no provider calls.
    assert!(provider.calls().is_empty(), "got {:?}", provider.calls());
}

#[tokio::test]
async fn information_url_resolves_active_provider_link_via_get_not_fuzzy() {
    // Hana-Kimi shape: the description body has no links, the Information
    // field points straight at the active provider's own series page
    // (mangabaka.org/{id}), and the title would fuzzy-match a wrong but
    // same-named series. The Information link must drive a direct get().
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mangabaka"));
    provider.register_get(sample_metadata());
    // A plausible-but-wrong fuzzy candidate exists for the title.
    provider.register_search(
        "Hana-Kimi",
        vec![SearchHit {
            external_id: "99999".into(),
            title: "The Art of Hana-Kimi".into(),
            year: None,
            cover_url: None,
            kind: Some(SeriesKind::Manga),
            score: None,
        }],
    );
    let mut b = MetadataRegistry::builder();
    b.register(provider.clone()).unwrap();
    b.set_active("mangabaka");
    let registry = Arc::new(b.build().unwrap());

    insert_release_full(
        &db,
        "r-info-direct",
        "Hana-Kimi",
        None,
        Some("https://mangabaka.org/12345"),
        &["cbz"],
    )
    .await;

    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r-info-direct").await.unwrap();

    assert_eq!(out.path, Some(ResolutionPath::ForeignIdLookup));
    assert!(out.series_id.is_some());
    assert_eq!(out.confidence, Some(1.0));
    let calls = provider.calls();
    assert!(
        calls.iter().any(|c| c == "get(12345)"),
        "expected get(12345); got {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.starts_with("search(")),
        "fuzzy search should not run when the Information link resolves; got {calls:?}"
    );

    let stored = releases::Entity::find_by_id("r-info-direct".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.resolution_status, "resolved");
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
            kind: None,
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
        total_volumes: None,
        total_chapters: None,
        rating: None,
        description: None,
        genres: vec![],
        tags: vec![],
        foreign_ids: vec![],
        raw: serde_json::json!({"id": 77777}),
        published_start_date: None,
        published_end_date: None,
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
            kind: None,
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
        total_volumes: None,
        total_chapters: None,
        rating: None,
        description: None,
        genres: vec![],
        tags: vec![],
        foreign_ids: vec![],
        raw: serde_json::json!({"id": 88888}),
        published_start_date: None,
        published_end_date: None,
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
            kind: None,
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
            kind: None,
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

/// Two same-titled candidates exist on the provider (a manga and a
/// novel adaptation, both at score ~1.0). For a CBZ-only release, the
/// resolver must pick the manga — even if the novel sits first in the
/// search results. This is the scenario from the Vexations-of-a-Shut-In-
/// Vampire-Princess bug report.
#[tokio::test]
async fn fuzzy_title_with_cbz_prefers_manga_over_same_titled_novel() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mb"));
    let manga = SeriesMetadata {
        external_id: "manga-1".into(),
        canonical_title: "Vampire Princess".into(),
        kind: Some(SeriesKind::Manga),
        published_start_date: None,
        published_end_date: None,
        content_hash: "h-m".into(),
        ..sample_metadata()
    };
    let novel = SeriesMetadata {
        external_id: "novel-1".into(),
        canonical_title: "Vampire Princess".into(),
        kind: Some(SeriesKind::Novel),
        published_start_date: None,
        published_end_date: None,
        content_hash: "h-n".into(),
        ..novel_metadata()
    };
    provider.register_get(manga.clone());
    provider.register_get(novel.clone());
    // Both score 1.0. Order matters: the novel comes FIRST in the
    // unfiltered ranking, so naive "best wins" picks it. Format-aware
    // filtering must demote it.
    provider.register_search(
        "Vampire Princess",
        vec![
            SearchHit {
                external_id: "novel-1".into(),
                title: "Vampire Princess".into(),
                year: None,
                cover_url: None,
                kind: Some(SeriesKind::Novel),
                score: Some(1.0),
            },
            SearchHit {
                external_id: "manga-1".into(),
                title: "Vampire Princess".into(),
                year: None,
                cover_url: None,
                kind: Some(SeriesKind::Manga),
                score: Some(1.0),
            },
        ],
    );
    let registry = build_registry(provider.clone());
    insert_release(&db, "r-cbz-prefer", "Vampire Princess", None, &["cbz"]).await;

    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r-cbz-prefer").await.unwrap();
    assert_eq!(out.path, Some(ResolutionPath::FuzzyTitle));
    assert_eq!(out.status, td_resolution::ResolutionStatus::Resolved);
    let stored = releases::Entity::find_by_id("r-cbz-prefer".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let linked_series_id = stored.series_id.expect("series should be linked");
    let linked = series::Entity::find_by_id(linked_series_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(linked.kind.as_deref(), Some("manga"));
}

/// When the release fires multiple format-type rules (cbz AND epub) and
/// confident candidates exist in *both* the manga bucket and the novel
/// bucket, the resolver refuses to guess: it routes the row to
/// `review_pending` with one candidate per bucket so the operator can
/// choose.
#[tokio::test]
async fn mixed_format_release_with_candidates_in_two_kinds_goes_to_review() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mb"));
    let manga = SeriesMetadata {
        external_id: "manga-2".into(),
        canonical_title: "Dual Form".into(),
        kind: Some(SeriesKind::Manga),
        published_start_date: None,
        published_end_date: None,
        content_hash: "h-mm".into(),
        ..sample_metadata()
    };
    let novel = SeriesMetadata {
        external_id: "novel-2".into(),
        canonical_title: "Dual Form".into(),
        kind: Some(SeriesKind::Novel),
        published_start_date: None,
        published_end_date: None,
        content_hash: "h-nn".into(),
        ..novel_metadata()
    };
    provider.register_get(manga);
    provider.register_get(novel);
    provider.register_search(
        "Dual Form",
        vec![
            SearchHit {
                external_id: "manga-2".into(),
                title: "Dual Form".into(),
                year: None,
                cover_url: None,
                kind: Some(SeriesKind::Manga),
                score: Some(1.0),
            },
            SearchHit {
                external_id: "novel-2".into(),
                title: "Dual Form".into(),
                year: None,
                cover_url: None,
                kind: Some(SeriesKind::Novel),
                score: Some(1.0),
            },
        ],
    );
    let registry = build_registry(provider.clone());
    insert_release(&db, "r-mixed", "Dual Form", None, &["cbz", "epub"]).await;

    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r-mixed").await.unwrap();
    assert_eq!(out.status, td_resolution::ResolutionStatus::ReviewPending);
    assert_eq!(out.series_id, None);
    assert_eq!(out.path, None);
    assert!(
        out.reason
            .as_deref()
            .unwrap()
            .contains("mixed_format_multi_kind")
    );
    let candidates = review_candidates::Entity::find()
        .filter(review_candidates::Column::ReleaseId.eq("r-mixed"))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(candidates.len(), 2);
}

/// When the only fuzzy candidate is format-incompatible (e.g. a CBZ
/// release whose sole same-titled match is a novel), the resolver still
/// links the release to that candidate but demotes it to `ambiguous`,
/// so the operator sees it in the review queue with the mismatch
/// reason. Filtering must not silently drop the candidate.
#[tokio::test]
async fn fuzzy_title_with_only_incompatible_candidate_demotes_to_ambiguous() {
    let db = fresh_db().await;
    let provider = Arc::new(FakeProvider::new("mb"));
    let novel = SeriesMetadata {
        external_id: "novel-3".into(),
        canonical_title: "Only Novel".into(),
        kind: Some(SeriesKind::Novel),
        published_start_date: None,
        published_end_date: None,
        content_hash: "h-only".into(),
        ..novel_metadata()
    };
    provider.register_get(novel);
    provider.register_search(
        "Only Novel",
        vec![SearchHit {
            external_id: "novel-3".into(),
            title: "Only Novel".into(),
            year: None,
            cover_url: None,
            kind: Some(SeriesKind::Novel),
            score: Some(1.0),
        }],
    );
    let registry = build_registry(provider.clone());
    insert_release(&db, "r-only-novel", "Only Novel", None, &["cbz"]).await;

    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r-only-novel").await.unwrap();
    assert_eq!(out.path, Some(ResolutionPath::FuzzyTitle));
    assert_eq!(out.status, td_resolution::ResolutionStatus::Ambiguous);
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
            published_start_date: None,
            published_end_date: None,
            content_hash: "h-a".into(),
            ..sample_metadata()
        },
    );
    b.register_foreign(
        "mangaupdates",
        "x",
        SeriesMetadata {
            external_id: "from-b".into(),
            published_start_date: None,
            published_end_date: None,
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
async fn resolve_all_picks_up_resolved_rows_but_skips_manual_links() {
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
    // Three releases all currently `resolved`:
    // - auto: previously matched by the resolver, eligible for re-resolve.
    // - manual: operator-linked via /releases/{id}/link, must be skipped.
    // - rejected: out of bounds entirely.
    insert_release(&db, "auto", "auto", Some(&links.to_string()), &["cbz"]).await;
    insert_release(&db, "manual", "manual", Some(&links.to_string()), &["cbz"]).await;
    insert_release(
        &db,
        "rejected",
        "rejected",
        Some(&links.to_string()),
        &["cbz"],
    )
    .await;
    let auto_row = releases::ActiveModel {
        id: Set("auto".into()),
        resolution_status: Set("resolved".into()),
        resolution_path: Set(Some("foreign_id_lookup".into())),
        ..Default::default()
    };
    releases::Entity::update(auto_row).exec(&db).await.unwrap();
    let manual_row = releases::ActiveModel {
        id: Set("manual".into()),
        resolution_status: Set("resolved".into()),
        resolution_path: Set(Some("manual".into())),
        ..Default::default()
    };
    releases::Entity::update(manual_row)
        .exec(&db)
        .await
        .unwrap();
    let rejected_row = releases::ActiveModel {
        id: Set("rejected".into()),
        resolution_status: Set("rejected".into()),
        resolution_path: Set(Some("rejected".into())),
        ..Default::default()
    };
    releases::Entity::update(rejected_row)
        .exec(&db)
        .await
        .unwrap();

    let resolver = make_resolver(&db, registry, ingestion_default());
    let summary = resolver.resolve_all(100).await.unwrap();
    assert_eq!(summary.resolved, 1, "only the auto-resolved row re-runs");
    assert_eq!(summary.errors, 0);

    let auto = releases::Entity::find_by_id("auto".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(auto.resolution_attempts, 1);

    let manual = releases::Entity::find_by_id("manual".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(manual.resolution_attempts, 0, "manual link must not retry");
    assert_eq!(manual.resolution_path.as_deref(), Some("manual"));

    let rejected = releases::Entity::find_by_id("rejected".to_string())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rejected.resolution_attempts, 0,
        "rejected row must not retry"
    );
    assert_eq!(rejected.resolution_status, "rejected");
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

/// Set an operator decision directly on a persisted release, the way the
/// reject / keep / manual-link handlers do.
async fn set_decision(
    db: &DatabaseConnection,
    id: &str,
    status: &str,
    path: Option<&str>,
    series_id: Option<i32>,
) {
    let row = releases::ActiveModel {
        id: Set(id.into()),
        resolution_status: Set(status.into()),
        resolution_path: Set(path.map(str::to_string)),
        series_id: Set(series_id),
        ..Default::default()
    };
    releases::Entity::update(row).exec(db).await.unwrap();
}

async fn stored_status(db: &DatabaseConnection, id: &str) -> (String, Option<String>, Option<i32>) {
    let row = releases::Entity::find_by_id(id.to_string())
        .one(db)
        .await
        .unwrap()
        .unwrap();
    (row.resolution_status, row.resolution_path, row.series_id)
}

/// An automatic resolve (poll, backfill, series search) must never overwrite a
/// decision the operator made by hand. Without the guard, a release carried by
/// two overlapping feeds is re-persisted and re-resolved by the *other* feed on
/// its next tick, silently reverting the rejection.
#[tokio::test]
async fn automatic_resolve_leaves_rejected_releases_alone() {
    let db = fresh_db().await;
    insert_release(&db, "r-rej", "Chainsaw Man v1", None, &["cbz"]).await;
    set_decision(&db, "r-rej", "rejected", Some("rejected"), None).await;

    let provider = Arc::new(FakeProvider::new("mb"));
    let registry = build_registry(provider.clone());
    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r-rej").await.unwrap();

    assert!(out.skipped, "an operator-decided release must be skipped");
    assert_eq!(out.status, td_resolution::ResolutionStatus::Rejected);
    assert!(
        provider.calls().is_empty(),
        "a skipped resolve must not touch the provider: {:?}",
        provider.calls()
    );
    assert_eq!(
        stored_status(&db, "r-rej").await,
        ("rejected".into(), Some("rejected".into()), None),
    );
}

#[tokio::test]
async fn automatic_resolve_leaves_standalone_releases_alone() {
    let db = fresh_db().await;
    insert_release(&db, "r-kept", "Some Artbook", None, &["cbz"]).await;
    set_decision(&db, "r-kept", "standalone", Some("standalone"), None).await;

    let provider = Arc::new(FakeProvider::new("mb"));
    let registry = build_registry(provider.clone());
    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r-kept").await.unwrap();

    assert!(out.skipped);
    assert_eq!(out.status, td_resolution::ResolutionStatus::Standalone);
    assert_eq!(
        stored_status(&db, "r-kept").await,
        ("standalone".into(), Some("standalone".into()), None),
    );
}

/// A manual link is an operator decision even though its status is `resolved`:
/// re-resolving would silently relink it to whatever the fuzzy step now
/// prefers.
#[tokio::test]
async fn automatic_resolve_preserves_a_manual_link() {
    let db = fresh_db().await;
    db.execute_unprepared(
        "INSERT INTO series (id, canonical_title, metadata_source, metadata_fetched_at,\
         first_seen_at, last_release_at, owned) VALUES (7, 'Hand Picked', 'api', 0, 0, 0, 0)",
    )
    .await
    .unwrap();
    insert_release(&db, "r-man", "Chainsaw Man v1", None, &["cbz"]).await;
    set_decision(&db, "r-man", "resolved", Some("manual"), Some(7)).await;

    let provider = Arc::new(FakeProvider::new("mb"));
    let registry = build_registry(provider.clone());
    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r-man").await.unwrap();

    assert!(out.skipped);
    assert_eq!(
        stored_status(&db, "r-man").await,
        ("resolved".into(), Some("manual".into()), Some(7)),
        "the operator's link must survive",
    );
}

/// The guard protects against *automatic* runs only. "Re-resolve" on a kept or
/// rejected release is exactly how the operator pulls it back into the
/// pipeline, so an explicit retry has to override.
#[tokio::test]
async fn operator_triggered_resolve_overrides_the_guard() {
    let db = fresh_db().await;
    insert_release(&db, "r-force", "Chainsaw Man v1", None, &["cbz"]).await;
    set_decision(&db, "r-force", "rejected", Some("rejected"), None).await;

    let provider = Arc::new(FakeProvider::new("mb"));
    let registry = build_registry(provider.clone());
    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one_forced("r-force").await.unwrap();

    assert!(!out.skipped, "an explicit retry must actually run");
    assert_ne!(
        stored_status(&db, "r-force").await.0,
        "rejected",
        "the retry re-ran the pipeline and rewrote the status",
    );
}

/// An ordinary undecided release is untouched by the guard.
#[tokio::test]
async fn automatic_resolve_still_runs_for_undecided_releases() {
    let db = fresh_db().await;
    insert_release(&db, "r-plain", "Chainsaw Man v1", None, &["cbz"]).await;

    let provider = Arc::new(FakeProvider::new("mb"));
    let registry = build_registry(provider.clone());
    let resolver = make_resolver(&db, registry, ingestion_default());
    let out = resolver.resolve_one("r-plain").await.unwrap();
    assert!(!out.skipped);
}

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
            total_volumes: None,
            total_chapters: None,
            rating: None,
            description: None,
            genres: vec![],
            tags: vec![],
            foreign_ids: vec![ForeignId {
                provider: "mangaupdates".into(),
                id: "6z1uqw7".into(),
                url: None,
            }],
            raw: serde_json::json!({"id": 55555}),
            published_start_date: None,
            published_end_date: None,
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
