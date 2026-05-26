//! Repository round-trip tests against in-memory SQLite.
//!
//! Each test boots a fresh in-memory database via `td_db::connect_in_memory`
//! (which also runs migrations), so the cases are independent.

use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait, Set, Statement};
use td_db::entities::{
    provider_cache_state, release_formats, releases, review_candidates, series,
    series_external_ids, source_state,
};
use td_db::repos::{
    provider_cache_state_repo, releases_repo, review_repo, series_external_ids_repo, series_repo,
    sources_repo,
};

async fn fresh_db() -> DatabaseConnection {
    td_db::connect_in_memory()
        .await
        .expect("in-memory db should boot")
}

fn sample_series(title: &str, alternates: Option<&str>) -> series::ActiveModel {
    series::ActiveModel {
        canonical_title: Set(title.to_string()),
        alternate_titles_json: Set(alternates.map(str::to_string)),
        cover_url: Set(None),
        kind: Set(Some("manga".into())),
        status: Set(Some("ongoing".into())),
        year: Set(Some(2020)),
        genres_json: Set(None),
        metadata_json: Set(None),
        metadata_source: Set("offline_cache".into()),
        metadata_hash: Set(None),
        metadata_fetched_at: Set(1_700_000_000),
        first_seen_at: Set(1_700_000_000),
        last_release_at: Set(1_700_000_000),
        highest_volume: Set(None),
        highest_chapter: Set(None),
        owned: Set(0),
        ..Default::default()
    }
}

fn sample_release(id: &str, series_id: Option<i32>) -> releases::ActiveModel {
    releases::ActiveModel {
        id: Set(id.to_string()),
        source_kind: Set("nyaa".into()),
        source_name: Set("trusted".into()),
        external_id: Set(id.to_string()),
        title: Set("Some Release v01".into()),
        link: Set(format!("https://example/{id}")),
        magnet: Set(Some("magnet:?xt=urn:btih:deadbeef".into())),
        torrent_url: Set(None),
        ddl_url: Set(None),
        info_hash: Set(Some("deadbeef".into())),
        size_bytes: Set(Some(123_456_789)),
        files_json: Set(Some(r#"["Some Release v01.cbz"]"#.into())),
        description_html: Set(None),
        extracted_links_json: Set(None),
        posted_at: Set(1_700_000_000),
        observed_at: Set(1_700_000_100),
        series_id: Set(series_id),
        resolution_path: Set(None),
        resolution_confidence: Set(None),
        resolution_status: Set("unresolved".into()),
        resolution_attempts: Set(0),
        last_resolve_attempt_at: Set(None),
        volume_span_json: Set(None),
        chapter_span_json: Set(None),
    }
}

#[tokio::test]
async fn migrations_create_all_expected_tables() {
    let db = fresh_db().await;
    let backend = db.get_database_backend();
    let stmt = Statement::from_string(
        backend,
        "SELECT name FROM sqlite_master WHERE type IN ('table', 'view') ORDER BY name".to_string(),
    );
    let rows = db.query_all(stmt).await.unwrap();
    let names: Vec<String> = rows
        .iter()
        .map(|r| r.try_get::<String>("", "name").unwrap())
        .collect();

    for expected in [
        "provider_cache_state",
        "release_formats",
        "releases",
        "review_candidates",
        "series",
        "series_external_ids",
        "series_fts",
        "source_state",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "expected table `{expected}` to exist; got {names:?}"
        );
    }
    // The old name should not exist.
    assert!(
        !names.contains(&"mangabaka_offline".to_string()),
        "mangabaka_offline should have been replaced by provider_cache_state"
    );
}

#[tokio::test]
async fn series_upsert_returns_assigned_id_and_round_trips() {
    let db = fresh_db().await;

    let inserted = series_repo::upsert(&db, sample_series("Chainsaw Man", None))
        .await
        .unwrap();
    assert!(
        inserted.id >= 1,
        "autoincrement should assign a positive id"
    );

    let got = series_repo::find_by_id(&db, inserted.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.canonical_title, "Chainsaw Man");
    assert_eq!(got.kind.as_deref(), Some("manga"));
    assert_eq!(got.owned, 0);

    // Upsert by id overwrites mutable columns.
    let mut updated = sample_series("Chainsaw Man (Updated)", None);
    updated.id = Set(inserted.id);
    updated.last_release_at = Set(1_800_000_000);
    series_repo::upsert(&db, updated).await.unwrap();

    let got = series_repo::find_by_id(&db, inserted.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.canonical_title, "Chainsaw Man (Updated)");
    assert_eq!(got.last_release_at, 1_800_000_000);
}

#[tokio::test]
async fn series_external_ids_round_trip_and_lookup() {
    let db = fresh_db().await;
    let s = series_repo::upsert(&db, sample_series("Berserk", None))
        .await
        .unwrap();

    series_external_ids_repo::upsert(&db, s.id, "mangabaka", "1", None, 1_700_000_000)
        .await
        .unwrap();
    series_external_ids_repo::upsert(
        &db,
        s.id,
        "mangaupdates",
        "42",
        Some("https://mu/42"),
        1_700_000_000,
    )
    .await
    .unwrap();

    // Lookup by provider id resolves back to the internal series.
    assert_eq!(
        series_external_ids_repo::find_series_id(&db, "mangabaka", "1")
            .await
            .unwrap(),
        Some(s.id)
    );
    assert_eq!(
        series_external_ids_repo::find_series_id(&db, "mangaupdates", "42")
            .await
            .unwrap(),
        Some(s.id)
    );
    assert_eq!(
        series_external_ids_repo::find_series_id(&db, "anilist", "999")
            .await
            .unwrap(),
        None
    );

    let all = series_external_ids_repo::list_for_series(&db, s.id)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);

    // Re-upserting the same (provider, external_id) is idempotent.
    series_external_ids_repo::upsert(&db, s.id, "mangabaka", "1", None, 1_700_000_500)
        .await
        .unwrap();
    let all = series_external_ids_repo::list_for_series(&db, s.id)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn series_external_ids_cascade_on_series_delete() {
    let db = fresh_db().await;
    let s = series_repo::upsert(&db, sample_series("To Delete", None))
        .await
        .unwrap();
    series_external_ids_repo::upsert(&db, s.id, "mangabaka", "7", None, 1_700_000_000)
        .await
        .unwrap();

    series::Entity::delete_by_id(s.id).exec(&db).await.unwrap();

    let leftover = series_external_ids::Entity::find().all(&db).await.unwrap();
    assert!(
        leftover.is_empty(),
        "series_external_ids should cascade on series delete; got {leftover:?}"
    );
}

#[tokio::test]
async fn releases_upsert_is_idempotent_on_source_kind_and_external_id() {
    let db = fresh_db().await;

    releases_repo::upsert(&db, sample_release("nyaa-1", None))
        .await
        .unwrap();
    // Re-upserting the same (source_kind, external_id) must not create a duplicate.
    releases_repo::upsert(&db, sample_release("nyaa-1", None))
        .await
        .unwrap();

    let count = releases::Entity::find().all(&db).await.unwrap().len();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn release_formats_attached_and_listed() {
    let db = fresh_db().await;
    releases_repo::upsert(&db, sample_release("nyaa-2", None))
        .await
        .unwrap();

    releases_repo::add_format(&db, "nyaa-2", "cbz")
        .await
        .unwrap();
    releases_repo::add_format(&db, "nyaa-2", "cbz")
        .await
        .unwrap(); // idempotent
    releases_repo::add_format(&db, "nyaa-2", "epub")
        .await
        .unwrap();

    let mut formats = releases_repo::list_formats(&db, "nyaa-2").await.unwrap();
    formats.sort();
    assert_eq!(formats, vec!["cbz", "epub"]);
}

#[tokio::test]
async fn release_format_rows_cascade_on_release_delete() {
    let db = fresh_db().await;
    releases_repo::upsert(&db, sample_release("nyaa-3", None))
        .await
        .unwrap();
    releases_repo::add_format(&db, "nyaa-3", "cbz")
        .await
        .unwrap();

    releases::Entity::delete_by_id("nyaa-3".to_string())
        .exec(&db)
        .await
        .unwrap();

    let leftover = release_formats::Entity::find().all(&db).await.unwrap();
    assert!(
        leftover.is_empty(),
        "format rows should cascade on release delete; got {leftover:?}"
    );
}

#[tokio::test]
async fn set_resolution_updates_status_and_link() {
    let db = fresh_db().await;
    let s = series_repo::upsert(&db, sample_series("Some Series", None))
        .await
        .unwrap();
    releases_repo::upsert(&db, sample_release("nyaa-4", None))
        .await
        .unwrap();

    releases_repo::set_resolution(
        &db,
        "nyaa-4",
        Some(s.id),
        Some("foreign_id_lookup".into()),
        Some(1.0),
        "resolved",
        1_700_000_500,
    )
    .await
    .unwrap();

    let got = releases_repo::find_by_id(&db, "nyaa-4")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.series_id, Some(s.id));
    assert_eq!(got.resolution_path.as_deref(), Some("foreign_id_lookup"));
    assert_eq!(got.resolution_status, "resolved");
    assert_eq!(got.last_resolve_attempt_at, Some(1_700_000_500));
}

#[tokio::test]
async fn source_state_upsert_round_trip() {
    let db = fresh_db().await;

    let model = source_state::ActiveModel {
        source_kind: Set("nyaa".into()),
        source_name: Set("trusted".into()),
        etag: Set(Some("W/\"abc\"".into())),
        cursor: Set(None),
        last_polled_at: Set(Some(1_700_000_000)),
        last_success_at: Set(Some(1_700_000_000)),
        last_error: Set(None),
        last_summary: Set(Some("ok: 12 new".into())),
    };
    sources_repo::upsert(&db, model).await.unwrap();

    let got = sources_repo::get(&db, "nyaa", "trusted")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.etag.as_deref(), Some("W/\"abc\""));
    assert_eq!(got.last_summary.as_deref(), Some("ok: 12 new"));

    // Second upsert with new summary overwrites in place.
    let updated = source_state::ActiveModel {
        source_kind: Set("nyaa".into()),
        source_name: Set("trusted".into()),
        etag: Set(Some("W/\"def\"".into())),
        cursor: Set(None),
        last_polled_at: Set(Some(1_700_001_000)),
        last_success_at: Set(Some(1_700_001_000)),
        last_error: Set(None),
        last_summary: Set(Some("ok: 0 new".into())),
    };
    sources_repo::upsert(&db, updated).await.unwrap();
    let got = sources_repo::get(&db, "nyaa", "trusted")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.etag.as_deref(), Some("W/\"def\""));
    assert_eq!(got.last_summary.as_deref(), Some("ok: 0 new"));
}

#[tokio::test]
async fn review_candidates_replace_for_release() {
    let db = fresh_db().await;
    let s1 = series_repo::upsert(&db, sample_series("Candidate 1", None))
        .await
        .unwrap();
    let s2 = series_repo::upsert(&db, sample_series("Candidate 2", None))
        .await
        .unwrap();
    releases_repo::upsert(&db, sample_release("nyaa-5", None))
        .await
        .unwrap();

    let first = vec![
        review_candidates::ActiveModel {
            release_id: Set("nyaa-5".into()),
            series_id: Set(s1.id),
            score: Set(0.9),
            reason: Set(Some("fuzzy".into())),
        },
        review_candidates::ActiveModel {
            release_id: Set("nyaa-5".into()),
            series_id: Set(s2.id),
            score: Set(0.6),
            reason: Set(Some("fuzzy".into())),
        },
    ];
    review_repo::replace_for_release(&db, "nyaa-5", first)
        .await
        .unwrap();

    let listed = review_repo::list_for_release(&db, "nyaa-5").await.unwrap();
    assert_eq!(listed.len(), 2);
    // Sorted by score desc.
    assert!(listed[0].score >= listed[1].score);

    // Replace with a smaller set; old candidate (s2) should disappear.
    let second = vec![review_candidates::ActiveModel {
        release_id: Set("nyaa-5".into()),
        series_id: Set(s1.id),
        score: Set(0.95),
        reason: Set(Some("fuzzy".into())),
    }];
    review_repo::replace_for_release(&db, "nyaa-5", second)
        .await
        .unwrap();

    let listed = review_repo::list_for_release(&db, "nyaa-5").await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].series_id, s1.id);
    assert!((listed[0].score - 0.95).abs() < 1e-9);
}

#[tokio::test]
async fn provider_cache_state_appends_and_returns_latest() {
    let db = fresh_db().await;
    let row = provider_cache_state_repo::append(
        &db,
        "mangabaka",
        1_700_000_000,
        Some("2026-05-01"),
        Some(123_456),
        Some("https://mangabaka/dump.tar.gz"),
        Some(987_654_321),
    )
    .await
    .unwrap();
    assert!(row.id >= 1);

    // A second refresh produces a second row; latest() picks the newest.
    provider_cache_state_repo::append(
        &db,
        "mangabaka",
        1_700_010_000,
        Some("2026-05-08"),
        Some(124_000),
        Some("https://mangabaka/dump.tar.gz"),
        Some(988_000_000),
    )
    .await
    .unwrap();

    let latest = provider_cache_state_repo::latest(&db, "mangabaka")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.fetched_at, 1_700_010_000);
    assert_eq!(latest.cache_version.as_deref(), Some("2026-05-08"));

    // Other providers do not interfere.
    let other = provider_cache_state_repo::latest(&db, "anilist")
        .await
        .unwrap();
    assert!(other.is_none());

    let rows = provider_cache_state::Entity::find().all(&db).await.unwrap();
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn fts5_returns_series_matched_by_title_and_alternate_titles() {
    let db = fresh_db().await;

    let s1 = series_repo::upsert(&db, sample_series("Chainsaw Man", None))
        .await
        .unwrap();
    let s2 = series_repo::upsert(
        &db,
        sample_series("Berserk", Some(r#"["Berserk: Black Swordsman"]"#)),
    )
    .await
    .unwrap();
    let s3 = series_repo::upsert(&db, sample_series("Vinland Saga", None))
        .await
        .unwrap();

    let hits = series_repo::search_fts(&db, "chainsaw", 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, s1.id);

    // Alternate-title hit lives in the alternate_titles FTS column.
    let hits = series_repo::search_fts(&db, "swordsman", 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, s2.id);

    // Update-trigger keeps FTS in sync.
    let mut updated = sample_series("Vinland Saga", Some(r#"["Wikingr"]"#));
    updated.id = Set(s3.id);
    updated.last_release_at = Set(1_800_000_000);
    series_repo::upsert(&db, updated).await.unwrap();
    let hits = series_repo::search_fts(&db, "wikingr", 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, s3.id);
}
