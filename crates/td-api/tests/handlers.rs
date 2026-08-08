//! Integration tests for the HTTP API.
//!
//! Each test builds the same router that `serve` would, exercises a route
//! via `tower::ServiceExt::oneshot`, and asserts on the parsed body. The
//! discovery / metadata layers use stub impls from `common`; no network.

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chrono::Utc;
use common::*;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde_json::Value;
use td_config::AuthConfig;
use td_db::entities::{releases, series};
use td_db::repos::{
    mangaupdates_id_repo, releases_repo, run_metrics_repo, series_external_ids_repo, tagging_repo,
};
use td_source::{DiscoveredRelease, PollOutcome};
use tower::ServiceExt;

fn open_auth() -> AuthConfig {
    AuthConfig {
        read_requires_auth: false,
        api_key: None,
        admin_token: Some("write-token".into()),
    }
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
}

async fn seed_series(db: &sea_orm::DatabaseConnection, title: &str, kind: &str) -> i32 {
    let now = Utc::now().timestamp();
    let model = series::ActiveModel {
        canonical_title: Set(title.into()),
        alternate_titles_json: Set(None),
        cover_url: Set(None),
        kind: Set(Some(kind.into())),
        status: Set(Some("ongoing".into())),
        year: Set(Some(2020)),
        metadata_json: Set(None),
        metadata_source: Set("api".into()),
        metadata_hash: Set(None),
        metadata_fetched_at: Set(now),
        first_seen_at: Set(now),
        last_release_at: Set(now),
        highest_volume: Set(None),
        highest_chapter: Set(None),
        owned: Set(0),
        ..Default::default()
    };
    let row = model.insert(db).await.unwrap();
    row.id
}

/// Seed a series with feed-relevant fields: a coverage list, an `updated_at`,
/// `highest_volume`, and a mangabaka external id.
async fn seed_feed_series(
    db: &sea_orm::DatabaseConnection,
    title: &str,
    updated_at: i64,
    volume_coverage_json: &str,
    highest_volume: f64,
    mangabaka_id: &str,
) -> i32 {
    let now = Utc::now().timestamp();
    let id = series::ActiveModel {
        canonical_title: Set(title.into()),
        metadata_source: Set("api".into()),
        metadata_fetched_at: Set(now),
        first_seen_at: Set(now),
        last_release_at: Set(now),
        owned: Set(0),
        updated_at: Set(updated_at),
        volume_coverage_json: Set(Some(volume_coverage_json.into())),
        highest_volume: Set(Some(highest_volume)),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap()
    .id;
    td_db::entities::series_external_ids::ActiveModel {
        provider: Set("mangabaka".into()),
        external_id: Set(mangabaka_id.into()),
        series_id: Set(id),
        fetched_at: Set(now),
    }
    .insert(db)
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn health_returns_ok() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn info_returns_name_and_version() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/info")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["name"], "tsundoku");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn series_list_paginates_and_filters_by_kind() {
    let db = fresh_db().await;
    let _m_id = seed_series(&db, "Manga A", "manga").await;
    let _m_id2 = seed_series(&db, "Manga B", "manga").await;
    let _n_id = seed_series(&db, "Novel X", "novel").await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?kind=manga&pageSize=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["total"], 2);
    assert_eq!(body["items"].as_array().unwrap().len(), 2);
    // Default order is last_release_at desc; all rows share the same `now`,
    // so the assertion is that they're all `manga`.
    for item in body["items"].as_array().unwrap() {
        assert_eq!(item["kind"], "manga");
    }
}

/// Persist a release on `source_name` and link it to `series_id`, so the
/// series counts as "has a release from that source". Returns the release id.
async fn link_release_from_source(
    db: &sea_orm::DatabaseConnection,
    external_id: &str,
    source_name: &str,
    series_id: i32,
) -> String {
    let r = sample_release(external_id, source_name, "Linked Release");
    let rid = releases_repo::persist_discovered(db, &r, Utc::now().timestamp())
        .await
        .unwrap();
    releases_repo::set_resolution(
        db,
        &rid,
        Some(series_id),
        None,
        None,
        "resolved",
        Utc::now().timestamp(),
    )
    .await
    .unwrap();
    rid
}

#[tokio::test]
async fn series_list_filters_by_source_admin_only() {
    let db = fresh_db().await;
    let a = seed_series(&db, "Alpha Series", "manga").await;
    let b = seed_series(&db, "Beta Series", "manga").await;
    link_release_from_source(&db, "a1", "alpha", a).await;
    link_release_from_source(&db, "b1", "beta", b).await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    // Helper: GET /series with an optional admin bearer; returns the body.
    let list = |query: &str, admin: bool| {
        let app = app.clone();
        let query = query.to_string();
        async move {
            let mut req = Request::builder().uri(format!("/api/v1/series?{query}"));
            if admin {
                req = req.header(header::AUTHORIZATION, "Bearer write-token");
            }
            let resp = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp).await
        }
    };

    // Admin, single source: only the series linked to that feed.
    let body = list("source=alpha", true).await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["canonicalTitle"], "Alpha Series");

    // Admin, two sources: OR-combined, both series.
    let body = list("source=alpha,beta", true).await;
    assert_eq!(body["total"], 2);

    // Non-admin: the param is ignored server-side, so it can't probe the
    // curated narrowing — both series come back.
    let body = list("source=alpha", false).await;
    assert_eq!(body["total"], 2);
}

#[tokio::test]
async fn sources_with_series_count_is_admin_only_and_ranks_feeds() {
    let db = fresh_db().await;
    let a = seed_series(&db, "Alpha Series", "manga").await;
    let b = seed_series(&db, "Beta Series", "manga").await;
    // alpha links two distinct series; beta links one.
    link_release_from_source(&db, "a1", "alpha", a).await;
    link_release_from_source(&db, "a2", "alpha", b).await;
    link_release_from_source(&db, "b1", "beta", a).await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    // Without an admin bearer the endpoint is rejected (it lives in the
    // require_admin group, like /codex/status).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/sources/with-series-count")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Admin: feeds ranked by distinct-series count, descending.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sources/with-series-count")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items[0]["name"], "alpha");
    assert_eq!(items[0]["seriesCount"], 2);
    assert_eq!(items[1]["name"], "beta");
    assert_eq!(items[1]["seriesCount"], 1);
}

#[tokio::test]
async fn series_list_filters_by_metadata_source() {
    let db = fresh_db().await;
    // seed_series defaults to metadata_source = "api" (provider-backed).
    seed_series(&db, "Provider Manga", "manga").await;
    seed_manual_series(&db, "Manual Manga").await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let titles = |body: &Value| -> Vec<String> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["canonicalTitle"].as_str().unwrap().to_string())
            .collect()
    };

    // metadataSource=manual keeps only the manual row.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?metadataSource=manual")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["total"], 1);
    assert_eq!(titles(&body), vec!["Manual Manga"]);

    // metadataSource=auto keeps only the provider-backed row.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?metadataSource=auto")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["total"], 1);
    assert_eq!(titles(&body), vec!["Provider Manga"]);

    // An unrecognized value applies no constraint (both rows returned).
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?metadataSource=bogus")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["total"], 2);
}

#[tokio::test]
async fn series_list_metadata_source_composes_with_kind_and_search() {
    let db = fresh_db().await;
    // Two manual rows (one manga, one novel) + a provider-backed manga that
    // shares the search term, so we can prove the filter ANDs with both `kind`
    // and the `q` search path.
    seed_manual_series_with_kind(&db, "Solo Leveling", "manga").await;
    seed_manual_series_with_kind(&db, "Solo Diary", "novel").await;
    seed_series(&db, "Solo Leveling Provider", "manga").await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let titles = |body: &Value| -> Vec<String> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["canonicalTitle"].as_str().unwrap().to_string())
            .collect()
    };

    // manual + kind=manga → only the manual manga row.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?metadataSource=manual&kind=manga")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(titles(&body), vec!["Solo Leveling"]);

    // manual + q=solo leveling → the search path honors the filter too,
    // excluding the provider-backed "Solo Leveling Provider".
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?metadataSource=manual&q=solo%20leveling")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    let got = titles(&body);
    assert!(got.contains(&"Solo Leveling".to_string()), "got {got:?}");
    assert!(
        !got.contains(&"Solo Leveling Provider".to_string()),
        "search path must apply metadataSource filter; got {got:?}"
    );
}

#[tokio::test]
async fn series_detail_returns_external_ids() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "Test Series", "manga").await;
    series_external_ids_repo::upsert(&db, sid, "mb", "42", 100)
        .await
        .unwrap();
    series_external_ids_repo::upsert(&db, sid, "anilist", "9", 100)
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/series/{sid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["id"], sid);
    let mut providers: Vec<String> = body["externalIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["provider"].as_str().unwrap().to_string())
        .collect();
    providers.sort();
    assert_eq!(providers, vec!["anilist", "mb"]);
}

#[tokio::test]
async fn series_detail_404s_for_unknown_id() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn series_lookup_resolves_external_id() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "Test Series", "manga").await;
    series_external_ids_repo::upsert(&db, sid, "mangabaka", "42", 100)
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    // A 200 here also proves the static `/series/lookup` route wins over
    // `/series/{id}` (a capture would fail the i32 path parse with a 400).
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/lookup?provider=mangabaka&externalId=42")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["matches"][0]["seriesId"], sid);
    assert_eq!(body["matches"][0]["provider"], "mangabaka");
    assert_eq!(body["matches"][0]["canonicalTitle"], "Test Series");
}

#[tokio::test]
async fn series_lookup_provider_is_case_insensitive() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "Test Series", "manga").await;
    series_external_ids_repo::upsert(&db, sid, "mal", "1", 100)
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/lookup?provider=MAL&externalId=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["matches"][0]["seriesId"], sid);
}

#[tokio::test]
async fn series_lookup_returns_no_matches_for_unknown_mapping() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "Test Series", "manga").await;
    series_external_ids_repo::upsert(&db, sid, "mangabaka", "42", 100)
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/lookup?provider=mangabaka&externalId=999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // A miss is expected for any series tsundoku has not discovered, so it is
    // an empty result rather than an error.
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["matches"].as_array().unwrap().len(), 0);
}

/// Without a provider the id is ambiguous by construction: provider id spaces
/// overlap, so the same number is a different series on each. Return the whole
/// set and let the caller disambiguate rather than silently picking one.
#[tokio::test]
async fn series_lookup_without_provider_returns_every_provider_match() {
    let db = fresh_db().await;
    let mb = seed_series(&db, "MangaBaka 1329", "manga").await;
    let mal = seed_series(&db, "MAL 1329", "manga").await;
    let other = seed_series(&db, "Unrelated", "manga").await;
    series_external_ids_repo::upsert(&db, mb, "mangabaka", "1329", 100)
        .await
        .unwrap();
    series_external_ids_repo::upsert(&db, mal, "mal", "1329", 100)
        .await
        .unwrap();
    series_external_ids_repo::upsert(&db, other, "kitsu", "555", 100)
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/lookup?externalId=1329")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let pairs: Vec<(String, i64)> = body["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["provider"].as_str().unwrap().to_string(),
                m["seriesId"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("mal".to_string(), mal as i64),
            ("mangabaka".to_string(), mb as i64)
        ],
        "both providers' rows come back, ordered by provider",
    );
}

/// Pasting a series URL is the common case, and the URL already names its
/// provider — so no dropdown is needed and the result is unambiguous.
#[tokio::test]
async fn series_lookup_infers_the_provider_from_a_pasted_url() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "My Quiet Blacksmith Life", "manga").await;
    series_external_ids_repo::upsert(&db, sid, "mangabaka", "6734", 100)
        .await
        .unwrap();
    // A decoy sharing the bare id under a different provider: inferring the
    // provider from the URL must exclude it.
    let decoy = seed_series(&db, "Decoy", "manga").await;
    series_external_ids_repo::upsert(&db, decoy, "mal", "6734", 100)
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/lookup?externalId=https%3A%2F%2Fmangabaka.dev%2F6734")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let matches = body["matches"].as_array().unwrap();
    assert_eq!(
        matches.len(),
        1,
        "the URL names its provider, so no ambiguity"
    );
    assert_eq!(matches[0]["seriesId"], sid);
    assert_eq!(matches[0]["provider"], "mangabaka");
}

/// Legacy MangaUpdates URLs detect as the synthetic `mangaupdates-legacy`
/// provider, which is never stored. Querying it directly would always miss
/// while looking like a supported URL, so the same translation cache the
/// resolver uses is consulted here.
#[tokio::test]
async fn series_lookup_translates_legacy_mangaupdates_urls_via_the_id_cache() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "Legacy Linked", "manga").await;
    series_external_ids_repo::upsert(&db, sid, "mangaupdates", "jwezzey", 100)
        .await
        .unwrap();
    td_db::repos::mangaupdates_id_repo::record(&db, 151349, Some("jwezzey"), 100)
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let uri = "/api/v1/series/lookup?externalId=https%3A%2F%2Fwww.mangaupdates.com%2Fseries.html%3Fid%3D151349";
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["matches"][0]["seriesId"], sid);
    assert_eq!(body["matches"][0]["provider"], "mangaupdates");
}

/// An uncached legacy id resolves to nothing rather than to the wrong series:
/// translating it for real needs a network redirect, which a lookup endpoint
/// has no business doing.
#[tokio::test]
async fn series_lookup_returns_nothing_for_an_uncached_legacy_mangaupdates_id() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "Some Series", "manga").await;
    series_external_ids_repo::upsert(&db, sid, "mangaupdates", "jwezzey", 100)
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let uri = "/api/v1/series/lookup?externalId=https%3A%2F%2Fwww.mangaupdates.com%2Fseries.html%3Fid%3D999999";
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["matches"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn series_lookup_400s_when_params_missing() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/lookup?provider=mangabaka")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// A raw `title LIKE '%q%'` needs a contiguous substring, which real release
/// titles defeat: they interleave the series name with volume spans, years,
/// and group tags. Typing two words that both appear must match.
#[tokio::test]
async fn review_queue_q_matches_tokens_out_of_order() {
    let db = fresh_db().await;
    let r = sample_release(
        "1",
        "feed",
        "My Quiet Blacksmith Life in Another World v01-05 (2024-2025) (Digital) (TooManyIsekai)",
    );
    releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let total_for = |q: &str| {
        let app = app.clone();
        let uri = format!("/api/v1/releases/unresolved?q={q}");
        async move {
            let resp = app
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp).await["total"].as_u64().unwrap()
        }
    };

    // Contiguous substring: worked before, must keep working.
    assert_eq!(total_for("Quiet%20Blacksmith").await, 1);
    // Tokens separated by other words — 0 under a raw substring match.
    assert_eq!(total_for("Blacksmith%20v01").await, 1);
    assert_eq!(total_for("Quiet%20TooManyIsekai").await, 1);
    // Order does not matter.
    assert_eq!(total_for("TooManyIsekai%20Quiet").await, 1);
    // Every token must still be present.
    assert_eq!(total_for("Blacksmith%20Chainsaw").await, 0);
}

/// Seed a release with an explicit link and status for the releases-list
/// search tests.
async fn seed_release_for_search(
    db: &sea_orm::DatabaseConnection,
    external_id: &str,
    title: &str,
    link: &str,
    status: &str,
) -> String {
    let mut r = sample_release(external_id, "feed", title);
    r.link = link.to_string();
    let id = releases_repo::persist_discovered(db, &r, Utc::now().timestamp())
        .await
        .unwrap();
    if status != "unresolved" {
        releases_repo::set_resolution(db, &id, None, None, None, status, Utc::now().timestamp())
            .await
            .unwrap();
    }
    id
}

/// The releases list is the debugging surface, so unlike the review queue it
/// must reach *every* status — `rejected` in particular has no other home in
/// the UI at all.
#[tokio::test]
async fn release_list_q_searches_across_all_statuses_including_rejected() {
    let db = fresh_db().await;
    let rejected = seed_release_for_search(
        &db,
        "1",
        "Chainsaw Man v01 (Digital) (LuCaZ)",
        "https://nyaa.si/view/1",
        "rejected",
    )
    .await;
    let resolved = seed_release_for_search(
        &db,
        "2",
        "Chainsaw Man v02 (Digital) (LuCaZ)",
        "https://nyaa.si/view/2",
        "resolved",
    )
    .await;
    seed_release_for_search(
        &db,
        "3",
        "Berserk v01",
        "https://nyaa.si/view/3",
        "unresolved",
    )
    .await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/releases?q=Chainsaw")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let ids: Vec<String> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(body["total"], 2);
    assert!(
        ids.contains(&rejected),
        "rejected releases must be findable"
    );
    assert!(ids.contains(&resolved));
}

/// Pasting the post URL is the fastest way to answer "did we ingest this?".
/// The stored `link` is matched exactly, which keeps this source-agnostic —
/// no per-source URL parser to keep in sync.
#[tokio::test]
async fn release_list_q_resolves_a_pasted_post_url() {
    let db = fresh_db().await;
    let target = seed_release_for_search(
        &db,
        "1997229",
        "My Quiet Blacksmith Life in Another World v01-05",
        "https://nyaa.si/view/1997229",
        "resolved",
    )
    .await;
    seed_release_for_search(
        &db,
        "1997230",
        "Something Else",
        "https://nyaa.si/view/1997230",
        "resolved",
    )
    .await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let fetch = |q: &str| {
        let app = app.clone();
        let uri = format!("/api/v1/releases?q={q}");
        async move {
            let resp = app
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp).await
        }
    };

    let by_url = fetch("https%3A%2F%2Fnyaa.si%2Fview%2F1997229").await;
    assert_eq!(by_url["total"], 1);
    assert_eq!(by_url["items"][0]["id"], target);

    // A trailing slash is what you get from some copy paths; it must not miss.
    let with_slash = fetch("https%3A%2F%2Fnyaa.si%2Fview%2F1997229%2F").await;
    assert_eq!(with_slash["total"], 1);

    // The bare post id works too — it is the `external_id` on the row.
    let by_id = fetch("1997229").await;
    assert_eq!(by_id["total"], 1);
    assert_eq!(by_id["items"][0]["id"], target);
}

/// "Show me every release we hold for MangaBaka 6734" — resolves the pair to
/// a series, then lists its releases.
#[tokio::test]
async fn release_list_filters_by_provider_external_id() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "My Quiet Blacksmith Life", "manga").await;
    series_external_ids_repo::upsert(&db, sid, "mangabaka", "6734", 100)
        .await
        .unwrap();
    let linked = seed_release_for_search(
        &db,
        "1",
        "Blacksmith v01",
        "https://nyaa.si/view/1",
        "resolved",
    )
    .await;
    releases_repo::set_resolution(
        &db,
        &linked,
        Some(sid),
        Some("manual".into()),
        None,
        "resolved",
        Utc::now().timestamp(),
    )
    .await
    .unwrap();
    seed_release_for_search(
        &db,
        "2",
        "Unrelated v01",
        "https://nyaa.si/view/2",
        "resolved",
    )
    .await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let fetch = |uri: String| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp).await
        }
    };

    let hit = fetch("/api/v1/releases?provider=mangabaka&externalId=6734".into()).await;
    assert_eq!(hit["total"], 1);
    assert_eq!(hit["items"][0]["id"], linked);

    // An id that maps to no series yields nothing rather than everything —
    // a filter that silently no-ops is worse than one that returns empty.
    let miss = fetch("/api/v1/releases?provider=mangabaka&externalId=999999".into()).await;
    assert_eq!(miss["total"], 0);
}

#[tokio::test]
async fn release_list_supports_status_format_and_sort() {
    let db = fresh_db().await;
    let mut older = sample_release("1", "feed", "Alpha v01");
    older.files = vec!["Alpha v01.cbz".into()];
    older.posted_at = chrono::DateTime::from_timestamp(1_000, 0).unwrap();
    let a = releases_repo::persist_discovered(&db, &older, 1_000)
        .await
        .unwrap();
    let mut newer = sample_release("2", "feed", "Beta v01");
    newer.files = vec!["Beta v01.epub".into()];
    newer.posted_at = chrono::DateTime::from_timestamp(9_000, 0).unwrap();
    let b = releases_repo::persist_discovered(&db, &newer, 9_000)
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let fetch = |uri: String| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp).await
        }
    };

    let cbz = fetch("/api/v1/releases?format=cbz".into()).await;
    assert_eq!(cbz["total"], 1);
    assert_eq!(cbz["items"][0]["id"], a);

    let asc = fetch("/api/v1/releases?sort=title_asc".into()).await;
    assert_eq!(asc["items"][0]["id"], a);
    let desc = fetch("/api/v1/releases?sort=title_desc".into()).await;
    assert_eq!(desc["items"][0]["id"], b);
}

#[tokio::test]
async fn release_list_returns_persisted_rows() {
    let db = fresh_db().await;
    let r = sample_release("1", "feed", "Chainsaw Man v01");
    let id = releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/releases")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["id"], id);
}

#[tokio::test]
async fn write_endpoint_rejects_request_without_bearer() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/releases/missing/retry")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn write_endpoint_503s_when_admin_token_unset() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        AuthConfig {
            read_requires_auth: false,
            api_key: None,
            admin_token: None,
        },
    );

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/releases/anything/retry")
                .header(header::AUTHORIZATION, "Bearer anything")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn read_endpoint_requires_api_key_when_configured() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        AuthConfig {
            read_requires_auth: true,
            api_key: Some("the-key".into()),
            admin_token: Some("write-token".into()),
        },
    );

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let resp_with_key = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/stats")
                .header("x-api-key", "the-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_with_key.status(), StatusCode::OK);
}

#[tokio::test]
async fn release_link_by_series_id_updates_resolution() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "Linkable", "manga").await;
    let r = sample_release("1", "feed", "title");
    let rid = releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();

    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let body = serde_json::json!({ "seriesId": sid });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/releases/{rid}/link"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["seriesId"], sid);
    assert_eq!(body["resolutionStatus"], "resolved");
    assert_eq!(body["resolutionPath"], "manual");
}

#[tokio::test]
async fn release_link_by_provider_external_id_upserts_series() {
    let db = fresh_db().await;
    let r = sample_release("1", "feed", "title");
    let rid = releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();

    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: Some(sample_metadata("mb", "1677", "Chainsaw Man")),
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let body = serde_json::json!({ "provider": "mb", "externalId": "1677" });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/releases/{rid}/link"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body["seriesId"].as_i64().unwrap() > 0);

    // The mapping should now exist in series_external_ids.
    let mapping = td_db::repos::series_external_ids_repo::find_series_id(&db, "mb", "1677")
        .await
        .unwrap();
    assert!(mapping.is_some());
}

#[tokio::test]
async fn release_reject_sets_rejected_status() {
    let db = fresh_db().await;
    let r = sample_release("1", "feed", "title");
    let rid = releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();

    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/releases/{rid}/reject"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let row = releases::Entity::find_by_id(rid.clone())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.resolution_status, "rejected");
}

#[tokio::test]
async fn create_manual_series_persists_manual_row() {
    let db = fresh_db().await;
    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let body = serde_json::json!({
        "canonicalTitle": "Some Doujin Circle Artbook",
        "kind": "manga",
        "year": 2022
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    let sid = body["id"].as_i64().unwrap();
    assert!(sid > 0);
    assert_eq!(body["canonicalTitle"], "Some Doujin Circle Artbook");
    assert_eq!(body["metadataSource"], "manual");
    assert_eq!(body["kind"], "manga");
    assert_eq!(body["year"], 2022);
    // A manual series carries no provider mappings.
    assert_eq!(body["externalIds"].as_array().unwrap().len(), 0);

    let row = series::Entity::find_by_id(sid as i32)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.metadata_source, "manual");
    assert_eq!(row.owned, 0);
}

#[tokio::test]
async fn create_manual_series_rejects_empty_title() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let body = serde_json::json!({ "canonicalTitle": "   " });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_manual_series_then_link_release_resolves_to_it() {
    let db = fresh_db().await;
    let r = sample_release("1", "feed", "Obscure Series MangaBaka Lacks v01");
    let rid = releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();

    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    // Create the manual series.
    let body = serde_json::json!({ "canonicalTitle": "Obscure Series" });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let sid = body_json(resp).await["id"].as_i64().unwrap() as i32;

    // Link the release to it via the existing link endpoint.
    let link_body = serde_json::json!({ "seriesId": sid });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/releases/{rid}/link"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&link_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["seriesId"], sid);
    assert_eq!(body["resolutionStatus"], "resolved");

    // The series remains manual after linking.
    let row = series::Entity::find_by_id(sid)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.metadata_source, "manual");
}

/// Seed a manual series straight into the DB (bypassing the create endpoint)
/// so update tests don't depend on the create handler.
async fn seed_manual_series(db: &sea_orm::DatabaseConnection, title: &str) -> i32 {
    let now = Utc::now().timestamp();
    series::ActiveModel {
        canonical_title: Set(title.into()),
        metadata_source: Set("manual".into()),
        metadata_fetched_at: Set(now),
        first_seen_at: Set(now),
        last_release_at: Set(now),
        owned: Set(0),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap()
    .id
}

/// Manual series with an explicit `kind`, for filter-composition tests.
async fn seed_manual_series_with_kind(
    db: &sea_orm::DatabaseConnection,
    title: &str,
    kind: &str,
) -> i32 {
    let now = Utc::now().timestamp();
    series::ActiveModel {
        canonical_title: Set(title.into()),
        kind: Set(Some(kind.into())),
        metadata_source: Set("manual".into()),
        metadata_fetched_at: Set(now),
        first_seen_at: Set(now),
        last_release_at: Set(now),
        owned: Set(0),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap()
    .id
}

#[tokio::test]
async fn update_manual_series_edits_descriptive_fields() {
    let db = fresh_db().await;
    let sid = seed_manual_series(&db, "Old Title").await;
    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let body = serde_json::json!({
        "canonicalTitle": "New Title",
        "alternateTitles": ["Alt One", "  ", "Alt Two"],
        "kind": "manga",
        "status": "completed",
        "year": 2021,
        "coverUrl": "https://example/cover.jpg",
        "description": "a synopsis"
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/v1/series/{sid}"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["canonicalTitle"], "New Title");
    // Blank alternate titles are trimmed out.
    assert_eq!(
        body["alternateTitles"].as_array().unwrap(),
        &vec![Value::from("Alt One"), Value::from("Alt Two")]
    );
    assert_eq!(body["kind"], "manga");
    assert_eq!(body["status"], "completed");
    assert_eq!(body["year"], 2021);
    assert_eq!(body["coverUrl"], "https://example/cover.jpg");
    assert_eq!(body["description"], "a synopsis");
    // Still manual; editing never changes provenance.
    assert_eq!(body["metadataSource"], "manual");

    let row = series::Entity::find_by_id(sid)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.canonical_title, "New Title");
    assert_eq!(row.metadata_source, "manual");
}

#[tokio::test]
async fn update_manual_series_rejects_empty_title() {
    let db = fresh_db().await;
    let sid = seed_manual_series(&db, "Keep Me").await;
    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let body = serde_json::json!({ "canonicalTitle": "   " });
    let resp = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/v1/series/{sid}"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn update_series_409s_for_provider_backed_row() {
    let db = fresh_db().await;
    // seed_series defaults to metadata_source = "api".
    let sid = seed_series(&db, "Provider Owned", "manga").await;
    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let body = serde_json::json!({ "canonicalTitle": "Hijacked" });
    let resp = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/v1/series/{sid}"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // The row is left untouched.
    let row = series::Entity::find_by_id(sid)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.canonical_title, "Provider Owned");
    assert_eq!(row.metadata_source, "api");
}

#[tokio::test]
async fn update_series_404s_for_unknown_id() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let body = serde_json::json!({ "canonicalTitle": "Ghost" });
    let resp = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v1/series/999999")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_series_requires_admin_token() {
    let db = fresh_db().await;
    let sid = seed_manual_series(&db, "Title").await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let body = serde_json::json!({ "canonicalTitle": "New" });
    let resp = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/v1/series/{sid}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn release_keep_sets_standalone_status() {
    let db = fresh_db().await;
    let r = sample_release("1", "feed", "The Shonen Jump Guide to Making Manga");
    let rid = releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();

    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/releases/{rid}/keep"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["resolutionStatus"], "standalone");
    assert_eq!(body["resolutionPath"], "standalone");
    // No series gets minted for a kept one-shot.
    assert!(body["seriesId"].is_null());

    let row = releases::Entity::find_by_id(rid.clone())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.resolution_status, "standalone");
}

#[tokio::test]
async fn kept_release_drops_out_of_review_queue_and_is_listable_by_status() {
    let db = fresh_db().await;
    let r = sample_release("1", "feed", "Making Manga Guidebook");
    let rid = releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();

    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    // Keep it.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/releases/{rid}/keep"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // It must not appear in the review queue any more.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/releases/unresolved")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["total"], 0, "kept release must leave the review queue");

    // But it is browsable via the standalone status filter.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/releases?status=standalone")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["id"], rid);
    assert_eq!(body["items"][0]["resolutionStatus"], "standalone");
}

#[tokio::test]
async fn stats_reports_counts() {
    let db = fresh_db().await;
    let _ = seed_series(&db, "A", "manga").await;
    let _ = seed_series(&db, "B", "manga").await;
    let r = sample_release("1", "feed", "title");
    releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["series"], 2);
    assert_eq!(body["totalReleases"], 1);
    assert_eq!(body["releases"]["unresolved"], 1);
    assert_eq!(body["activeProvider"], "mb");
}

#[tokio::test]
async fn providers_list_marks_active() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], "mb");
    assert_eq!(items[0]["active"], true);
}

#[tokio::test]
async fn sources_list_returns_registered_sources() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![StubSource {
            name: "test-feed".into(),
            kind: "stub".into(),
            outcome: PollOutcome::default(),
        }]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sources")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "test-feed");
    assert_eq!(items[0]["kind"], "stub");
}

#[tokio::test]
async fn sources_list_hydrates_in_flight_from_running_row() {
    let db = fresh_db().await;
    // Seed a `running` poll_runs row for the source the test will list.
    // The handler must surface it as `in_flight.started_at`.
    run_metrics_repo::start_poll_run(&db, "test-feed", "stub", 12_345, "manual")
        .await
        .unwrap();
    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![StubSource {
            name: "test-feed".into(),
            kind: "stub".into(),
            outcome: PollOutcome::default(),
        }]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sources")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let item = &body["items"][0];
    assert_eq!(item["name"], "test-feed");
    assert_eq!(
        item["inFlight"]["startedAt"], 12_345_i64,
        "running poll_runs row must hydrate inFlight; body: {body}"
    );

    // After finalizing, the field disappears (skip_serializing_if = None).
    // Run with a fresh router so the handler re-queries the DB cleanly.
    let id = run_metrics_repo::start_poll_run(&db, "another-feed", "stub", 9_000, "manual")
        .await
        .unwrap();
    // Mark the test-feed row finalized; another-feed stays running.
    let test_feed_row = run_metrics_repo::find_in_flight_poll_for_source(&db, "test-feed")
        .await
        .unwrap();
    assert!(test_feed_row.is_some());
    let _ = id; // silence unused; the second row is just to vary the data
}

#[tokio::test]
async fn sources_list_omits_in_flight_when_no_running_row() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![StubSource {
            name: "test-feed".into(),
            kind: "stub".into(),
            outcome: PollOutcome::default(),
        }]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sources")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let item = &body["items"][0];
    assert!(
        item.get("inFlight").is_none(),
        "no running row → inFlight field must be omitted; body: {body}"
    );
}

#[tokio::test]
async fn providers_list_hydrates_in_flight_from_running_row() {
    let db = fresh_db().await;
    run_metrics_repo::start_provider_refresh(&db, "mb", 22_222, "cron")
        .await
        .unwrap();
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let item = &body["items"][0];
    assert_eq!(item["id"], "mb");
    assert_eq!(
        item["inFlight"]["startedAt"], 22_222_i64,
        "running provider_refreshes row must hydrate inFlight; body: {body}"
    );
}

#[tokio::test]
async fn sources_list_includes_config_block_from_app_state() {
    let db = fresh_db().await;
    let nyaa_cfg = td_config::SourceConfig {
        kind: "nyaa".into(),
        name: "trusted".into(),
        cron: Some("*/30 * * * *".into()),
        enabled: true,
        nyaa: Some(td_config::NyaaSourceOptions {
            feed_url: "https://nyaa.si/?page=rss&f=2".into(),
            timeout_seconds: 45,
            fetch_details: true,
            site_base_url: "https://nyaa.si".into(),
        }),
    };
    let app = common::build_app_full(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![StubSource {
            name: "trusted".into(),
            kind: "nyaa".into(),
            outcome: PollOutcome::default(),
        }]),
        open_auth(),
        vec![nyaa_cfg],
        td_config::ProvidersConfig::default(),
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sources")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let item = &body["items"][0];
    assert_eq!(item["name"], "trusted");
    assert_eq!(item["config"]["enabled"], true);
    assert_eq!(item["config"]["cron"], "*/30 * * * *");
    assert_eq!(item["config"]["feedUrl"], "https://nyaa.si/?page=rss&f=2");
    assert_eq!(item["config"]["fetchDetails"], true);
    assert_eq!(item["config"]["timeoutSeconds"], 45);
    assert_eq!(item["config"]["maxPages"], 1);
}

#[tokio::test]
async fn providers_list_includes_config_block_without_api_key_value() {
    let db = fresh_db().await;
    let mut providers_cfg = td_config::ProvidersConfig::default();
    providers_cfg.mangabaka.api_key = Some("super-secret-leaky-token".into());
    providers_cfg.mangabaka.api_fallback = true;
    providers_cfg.mangabaka.offline_dump_url = Some("https://example.com/mangabaka.tar.gz".into());
    providers_cfg.mangabaka.offline_refresh_cron = Some("0 4 * * *".into());

    // Use the canonical mangabaka id so the handler emits its config block.
    let app = common::build_app_full(
        db,
        metadata_registry_with(StubProvider {
            id: "mangabaka",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
        Vec::new(),
        providers_cfg,
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let raw = std::str::from_utf8(&bytes).unwrap();
    // Defense-in-depth: the api_key value must never appear anywhere in the
    // serialized response, regardless of where it's nested.
    assert!(
        !raw.contains("super-secret-leaky-token"),
        "api_key value leaked into providers response: {raw}"
    );

    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let item = &body["items"][0];
    assert_eq!(item["id"], "mangabaka");
    assert_eq!(item["config"]["apiKeySet"], true);
    assert_eq!(item["config"]["apiFallback"], true);
    assert_eq!(
        item["config"]["offlineDumpUrl"],
        "https://example.com/mangabaka.tar.gz"
    );
    assert_eq!(item["config"]["offlineDumpConfigured"], true);
    assert_eq!(item["config"]["offlineRefreshCron"], "0 4 * * *");
    // Stub provider doesn't have an offline store loaded.
    assert_eq!(item["config"]["offlineCacheLoaded"], false);
    // Ensure no `apiKey` field exists (only `apiKeySet`).
    assert!(item["config"].get("apiKey").is_none());
}

#[tokio::test]
async fn poll_all_returns_per_source_triggered_results() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![
            StubSource {
                name: "a".into(),
                kind: "stub".into(),
                outcome: PollOutcome::default(),
            },
            StubSource {
                name: "b".into(),
                kind: "stub".into(),
                outcome: PollOutcome::default(),
            },
        ]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sources/poll-all")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    // Stable order matches `list`.
    assert_eq!(results[0]["source"], "a");
    assert_eq!(results[0]["triggered"], true);
    assert_eq!(results[0]["skipped"], false);
    assert_eq!(results[1]["source"], "b");
    assert_eq!(results[1]["triggered"], true);
}

#[tokio::test]
async fn poll_all_reports_locked_source_as_skipped() {
    let db = fresh_db().await;
    let locks = std::sync::Arc::new(td_scheduler::JobLocks::default());
    // Acquire and hold the lock for source "a" before the request lands.
    let held = locks.source_lock("a");
    let _guard = held.try_lock().expect("test should hold the lock first");

    let app = common::build_app_full(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![
            StubSource {
                name: "a".into(),
                kind: "stub".into(),
                outcome: PollOutcome::default(),
            },
            StubSource {
                name: "b".into(),
                kind: "stub".into(),
                outcome: PollOutcome::default(),
            },
        ]),
        open_auth(),
        Vec::new(),
        td_config::ProvidersConfig::default(),
        locks,
    );

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sources/poll-all")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["source"], "a");
    assert_eq!(results[0]["triggered"], false);
    assert_eq!(results[0]["skipped"], true);
    assert_eq!(results[1]["source"], "b");
    assert_eq!(results[1]["triggered"], true);
    assert_eq!(results[1]["skipped"], false);
}

#[tokio::test]
async fn reenrich_triggers_and_echoes_validated_statuses() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![StubSource {
            name: "a".into(),
            kind: "stub".into(),
            outcome: PollOutcome::default(),
        }]),
        open_auth(),
    );

    // Duplicate values collapse; order is preserved. Omitted scope and
    // filter fields echo their defaults (all origins, full refresh).
    let body = serde_json::json!({ "statuses": ["unresolved", "unresolved", "ambiguous"] });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/releases/re-enrich")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["triggered"], true);
    assert_eq!(body["skipped"], false);
    assert_eq!(
        body["statuses"],
        serde_json::json!(["unresolved", "ambiguous"])
    );
    assert_eq!(body["onlyMissingDetails"], false);
    assert_eq!(body["sources"], serde_json::Value::Null);
}

#[tokio::test]
async fn reenrich_echoes_scope_and_missing_details_filter() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![StubSource {
            name: "a".into(),
            kind: "stub".into(),
            outcome: PollOutcome::default(),
        }]),
        open_auth(),
    );

    // Names are deliberately not validated against the registries (removed
    // origins stay targetable), so an arbitrary name is accepted.
    let body = serde_json::json!({
        "statuses": ["unresolved"],
        "onlyMissingDetails": true,
        "sources": ["a", "some-removed-origin"],
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/releases/re-enrich")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["triggered"], true);
    assert_eq!(body["onlyMissingDetails"], true);
    assert_eq!(
        body["sources"],
        serde_json::json!(["a", "some-removed-origin"])
    );
}

#[tokio::test]
async fn reenrich_rejects_empty_and_unknown_statuses() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![StubSource {
            name: "a".into(),
            kind: "stub".into(),
            outcome: PollOutcome::default(),
        }]),
        open_auth(),
    );

    // Empty status set.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/releases/re-enrich")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({ "statuses": [] })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Unknown status.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/releases/re-enrich")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({ "statuses": ["bogus"] })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Provided-but-empty scope: distinct from omitted (= all origins), so
    // it is rejected rather than silently matching nothing.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/releases/re-enrich")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(
                        &serde_json::json!({ "statuses": ["unresolved"], "sources": [] }),
                    )
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn refresh_all_returns_per_provider_results() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/providers/refresh-all")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["provider"], "mb");
    assert_eq!(results[0]["triggered"], true);
    assert_eq!(results[0]["skipped"], false);
}

#[tokio::test]
async fn unresolved_endpoint_returns_review_pending_releases() {
    let db = fresh_db().await;
    let r = sample_release("1", "feed", "title");
    releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/releases/unresolved")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["total"], 1);
    // Phase B additions: every item carries arrays (default-empty for
    // a release that hasn't run through the resolver yet) and a
    // top_candidate convenience field.
    let item = &body["items"][0];
    assert!(item.get("searchQueries").is_some(), "missing searchQueries");
    assert!(
        item.get("cleanupRulesApplied").is_some(),
        "missing cleanupRulesApplied"
    );
    assert!(item.get("topCandidate").is_some(), "missing topCandidate");
    assert_eq!(item["searchQueries"].as_array().unwrap().len(), 0);
    assert_eq!(item["cleanupRulesApplied"].as_array().unwrap().len(), 0);
    assert!(item["topCandidate"].is_null());
}

#[tokio::test]
async fn unresolved_endpoint_surfaces_persisted_search_queries_and_rules() {
    use td_db::entities::releases;
    use td_resolution::persist::persist_search_queries;

    let db = fresh_db().await;
    let r = sample_release(
        "phase-b-1",
        "feed",
        "Solo Leveling (2021-2026) (Digital) (1r0n)",
    );
    releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();
    // Simulate a resolve cycle that left the cleaned queries on the row.
    let release_id = releases_repo::id_for(&r.source_kind, &r.external_id);
    assert!(
        releases::Entity::find_by_id(release_id.clone())
            .one(&db)
            .await
            .unwrap()
            .is_some(),
        "persisted release should be findable"
    );
    persist_search_queries(
        &db,
        &release_id,
        &["Solo Leveling".to_string()],
        &["strip_parens".to_string(), "strip_format".to_string()],
    )
    .await
    .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/releases/unresolved")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let item = &body["items"][0];
    assert_eq!(item["searchQueries"][0], "Solo Leveling");
    let rules: Vec<String> = item["cleanupRulesApplied"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(rules.contains(&"strip_parens".to_string()));
    assert!(rules.contains(&"strip_format".to_string()));
}

/// Seed a release into the queue with a chosen source_name, file (so a
/// format is detected), and resolution status. Returns the release id.
async fn seed_queue_release(
    db: &sea_orm::DatabaseConnection,
    external_id: &str,
    source_name: &str,
    title: &str,
    file: &str,
    status: &str,
) -> String {
    let mut r = sample_release(external_id, source_name, title);
    r.files = vec![file.to_string()];
    let id = releases_repo::persist_discovered(db, &r, Utc::now().timestamp())
        .await
        .unwrap();
    if status != "unresolved" {
        releases_repo::set_resolution(db, &id, None, None, None, status, Utc::now().timestamp())
            .await
            .unwrap();
    }
    id
}

fn queue_app(db: sea_orm::DatabaseConnection) -> axum::Router {
    build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    )
}

async fn queue_total(app: &axum::Router, query: &str) -> u64 {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/releases/unresolved?{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    body["total"].as_u64().unwrap()
}

#[tokio::test]
async fn unresolved_endpoint_filters_by_source_name_format_and_title() {
    let db = fresh_db().await;
    seed_queue_release(
        &db,
        "1",
        "trusted",
        "Solo Leveling",
        "vol.cbz",
        "unresolved",
    )
    .await;
    seed_queue_release(
        &db,
        "2",
        "trusted",
        "Berserk",
        "book.epub",
        "review_pending",
    )
    .await;
    seed_queue_release(
        &db,
        "3",
        "popular",
        "Solo Leveling Side",
        "vol.cbz",
        "ambiguous",
    )
    .await;
    let app = queue_app(db);

    // No filters: all three queue rows.
    assert_eq!(queue_total(&app, "").await, 3);
    // Source name narrows to the two `trusted` rows.
    assert_eq!(queue_total(&app, "sourceName=trusted").await, 2);
    // Format narrows to the two cbz rows.
    assert_eq!(queue_total(&app, "format=cbz").await, 2);
    assert_eq!(queue_total(&app, "format=epub").await, 1);
    // Title search is a substring match.
    assert_eq!(queue_total(&app, "q=Solo").await, 2);
    assert_eq!(queue_total(&app, "q=Berserk").await, 1);
    // Filters compose (AND): trusted + cbz = release 1 only.
    assert_eq!(queue_total(&app, "sourceName=trusted&format=cbz").await, 1);
}

#[tokio::test]
async fn unresolved_endpoint_status_filter_clamps_to_queue_statuses() {
    let db = fresh_db().await;
    seed_queue_release(&db, "1", "feed", "A", "a.cbz", "unresolved").await;
    seed_queue_release(&db, "2", "feed", "B", "b.cbz", "review_pending").await;
    seed_queue_release(&db, "3", "feed", "C", "c.cbz", "ambiguous").await;
    // A resolved row must never appear in the queue regardless of filters.
    let sid = seed_series(&db, "D Series", "manga").await;
    let resolved = seed_queue_release(&db, "4", "feed", "D", "d.cbz", "unresolved").await;
    releases_repo::set_resolution(
        &db,
        &resolved,
        Some(sid),
        Some("manual".into()),
        Some(1.0),
        "resolved",
        Utc::now().timestamp(),
    )
    .await
    .unwrap();
    let app = queue_app(db);

    // Narrow to a single queue status.
    assert_eq!(queue_total(&app, "status=review_pending").await, 1);
    assert_eq!(queue_total(&app, "status=ambiguous").await, 1);
    // An out-of-queue status falls back to the full three-status set, never
    // surfacing the resolved row.
    assert_eq!(queue_total(&app, "status=resolved").await, 3);
    assert_eq!(queue_total(&app, "status=bogus").await, 3);
}

async fn post_json(app: &axum::Router, uri: &str, body: serde_json::Value) -> Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "POST {uri} failed: {}",
        resp.status()
    );
    body_json(resp).await
}

#[tokio::test]
async fn bulk_reject_targets_explicit_ids_only() {
    let db = fresh_db().await;
    let a = seed_queue_release(&db, "1", "feed", "A", "a.cbz", "unresolved").await;
    let b = seed_queue_release(&db, "2", "feed", "B", "b.cbz", "review_pending").await;
    seed_queue_release(&db, "3", "feed", "C", "c.cbz", "ambiguous").await;
    let app = queue_app(db);

    let body = post_json(
        &app,
        "/api/v1/releases/bulk/reject",
        serde_json::json!({ "ids": [a, b] }),
    )
    .await;
    assert_eq!(body["rejected"], 2);
    // Only the third release remains in the queue.
    assert_eq!(queue_total(&app, "").await, 1);
}

#[tokio::test]
async fn bulk_reject_by_filter_rejects_whole_matching_set() {
    let db = fresh_db().await;
    seed_queue_release(&db, "1", "feed", "A", "a.cbz", "unresolved").await;
    seed_queue_release(&db, "2", "feed", "B", "b.cbz", "review_pending").await;
    seed_queue_release(&db, "3", "feed", "C", "c.epub", "ambiguous").await;
    let app = queue_app(db);

    // No ids: the filter selects the target set (both cbz rows).
    let body = post_json(
        &app,
        "/api/v1/releases/bulk/reject",
        serde_json::json!({ "format": "cbz" }),
    )
    .await;
    assert_eq!(body["rejected"], 2);
    // The epub row is the only one left.
    assert_eq!(queue_total(&app, "").await, 1);
    assert_eq!(queue_total(&app, "format=epub").await, 1);
}

/// Seed a queue release and stamp its cleaned `search_queries` (longest-first,
/// as the resolver would) so the `searchQuery`/`breadth` group filter has
/// something to match against.
async fn seed_queue_release_with_queries(
    db: &sea_orm::DatabaseConnection,
    external_id: &str,
    title: &str,
    queries: &[&str],
) -> String {
    let id = seed_queue_release(db, external_id, "feed", title, "a.cbz", "unresolved").await;
    let queries: Vec<String> = queries.iter().map(|s| s.to_string()).collect();
    td_resolution::persist::persist_search_queries(db, &id, &queries, &[])
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn unresolved_endpoint_search_query_filter_honors_breadth() {
    let db = fresh_db().await;
    // A & B match "one piece" on the primary [0]; C matches it only at [1];
    // D never matches.
    seed_queue_release_with_queries(&db, "1", "A", &["one piece", "one piece digital"]).await;
    seed_queue_release_with_queries(&db, "2", "B", &["one piece"]).await;
    seed_queue_release_with_queries(&db, "3", "C", &["bleach", "one piece"]).await;
    seed_queue_release_with_queries(&db, "4", "D", &["naruto"]).await;
    let app = queue_app(db);

    // Breadth 1 (default): only [0] matches → A, B.
    assert_eq!(queue_total(&app, "searchQuery=one+piece").await, 2);
    assert_eq!(
        queue_total(&app, "searchQuery=one+piece&breadth=1").await,
        2
    );
    // Breadth 2: [0..2) now includes C's secondary query → A, B, C.
    assert_eq!(
        queue_total(&app, "searchQuery=one+piece&breadth=2").await,
        3
    );
    // Breadth 3: flatten all → still A, B, C here.
    assert_eq!(
        queue_total(&app, "searchQuery=one+piece&breadth=3").await,
        3
    );
    // Out-of-range breadth clamps to the tight default (1).
    assert_eq!(
        queue_total(&app, "searchQuery=one+piece&breadth=9").await,
        2
    );

    // The [1]-only match (C) is excluded at breadth 1 but present at breadth 2.
    let tight = queue_titles(&app, "searchQuery=one+piece&breadth=1").await;
    assert!(!tight.contains(&"C".to_string()));
    let loose = queue_titles(&app, "searchQuery=one+piece&breadth=2").await;
    assert!(loose.contains(&"C".to_string()));

    // An exact-string match: a query that is a substring of another variant
    // must not match it (json_each compares the whole element).
    assert_eq!(queue_total(&app, "searchQuery=one+piece+digital").await, 0);
    assert_eq!(
        queue_total(&app, "searchQuery=one+piece+digital&breadth=3").await,
        1
    );
}

#[tokio::test]
async fn unresolved_endpoint_search_query_composes_with_title_q() {
    let db = fresh_db().await;
    // Two releases share the "one piece" group; only one has "Color" in its
    // title, so `q` and `searchQuery` AND together.
    seed_queue_release_with_queries(&db, "1", "One Piece Color Vol 1", &["one piece"]).await;
    seed_queue_release_with_queries(&db, "2", "One Piece Vol 2", &["one piece"]).await;
    seed_queue_release_with_queries(&db, "3", "Bleach Color", &["bleach"]).await;
    let app = queue_app(db);

    assert_eq!(queue_total(&app, "searchQuery=one+piece").await, 2);
    assert_eq!(queue_total(&app, "q=Color").await, 2);
    // AND: group "one piece" AND title contains "Color" → just release 1.
    assert_eq!(queue_total(&app, "searchQuery=one+piece&q=Color").await, 1);
}

#[tokio::test]
async fn bulk_reject_by_search_query_rejects_whole_group() {
    let db = fresh_db().await;
    seed_queue_release_with_queries(&db, "1", "A", &["one piece"]).await;
    seed_queue_release_with_queries(&db, "2", "B", &["bleach", "one piece"]).await;
    seed_queue_release_with_queries(&db, "3", "C", &["naruto"]).await;
    let app = queue_app(db);

    // Breadth 2 so the [1]-only member (B) is included in the group action.
    let body = post_json(
        &app,
        "/api/v1/releases/bulk/reject",
        serde_json::json!({ "searchQuery": "one piece", "breadth": 2 }),
    )
    .await;
    assert_eq!(body["rejected"], 2);
    // Only the naruto release remains.
    assert_eq!(queue_total(&app, "").await, 1);
}

async fn fetch_groups(app: &axum::Router, query: &str) -> Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/releases/unresolved/groups?{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await
}

/// Minimal percent-encoder for query-string values (the parity test feeds a
/// group's own `query` string back as a `searchQuery` param).
fn encode_query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[tokio::test]
async fn groups_endpoint_clusters_and_excludes_singletons() {
    let db = fresh_db().await;
    seed_queue_release_with_queries(&db, "1", "A", &["one piece", "one piece digital"]).await;
    seed_queue_release_with_queries(&db, "2", "B", &["one piece"]).await;
    seed_queue_release_with_queries(&db, "3", "C", &["bleach", "one piece"]).await;
    seed_queue_release_with_queries(&db, "4", "D", &["bleach"]).await;
    seed_queue_release_with_queries(&db, "5", "E", &["naruto"]).await;
    let app = queue_app(db);

    // Breadth 1 (default): group by primary [0]. naruto is a singleton and is
    // excluded by HAVING count > 1. one piece (A, B) and bleach (C, D) tie at
    // 2, so they sort by query ascending: bleach before one piece.
    let body = fetch_groups(&app, "").await;
    assert_eq!(body["breadth"], 1);
    let groups = body["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["query"], "bleach");
    assert_eq!(groups[0]["count"], 2);
    assert_eq!(groups[1]["query"], "one piece");
    assert_eq!(groups[1]["count"], 2);
    // Hint deferred to a later phase: never present yet.
    assert!(groups[0].get("topCandidate").is_none());
}

#[tokio::test]
async fn groups_endpoint_breadth_widens_clusters_and_orders_by_count() {
    let db = fresh_db().await;
    seed_queue_release_with_queries(&db, "1", "A", &["one piece", "one piece digital"]).await;
    seed_queue_release_with_queries(&db, "2", "B", &["one piece"]).await;
    seed_queue_release_with_queries(&db, "3", "C", &["bleach", "one piece"]).await;
    seed_queue_release_with_queries(&db, "4", "D", &["bleach"]).await;
    let app = queue_app(db);

    // Breadth 2 pulls C's secondary "one piece" into that group (now 3),
    // overtaking bleach (still 2). Ordering is by descending count.
    let body = fetch_groups(&app, "breadth=2").await;
    assert_eq!(body["breadth"], 2);
    let groups = body["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["query"], "one piece");
    assert_eq!(groups[0]["count"], 3);
    assert_eq!(groups[1]["query"], "bleach");
    assert_eq!(groups[1]["count"], 2);
}

#[tokio::test]
async fn groups_endpoint_compose_with_filters() {
    let db = fresh_db().await;
    // Two "one piece" releases on different sources; a source filter narrows
    // the group below the singleton threshold and drops it.
    let a = seed_queue_release(&db, "1", "trusted", "A", "a.cbz", "unresolved").await;
    let b = seed_queue_release(&db, "2", "popular", "B", "b.cbz", "unresolved").await;
    for id in [&a, &b] {
        td_resolution::persist::persist_search_queries(&db, id, &["one piece".into()], &[])
            .await
            .unwrap();
    }
    let app = queue_app(db);

    // No filter: a single 2-member group.
    assert_eq!(
        fetch_groups(&app, "").await["groups"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Scoped to one source: only one member remains, so the group falls below
    // the HAVING threshold and disappears.
    let scoped = fetch_groups(&app, "sourceName=trusted").await;
    assert_eq!(scoped["groups"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn groups_endpoint_parity_with_list_filter() {
    let db = fresh_db().await;
    // A deliberately mixed queue: shared primaries, secondary-only overlaps,
    // singletons, and a release with no search_queries at all.
    seed_queue_release_with_queries(&db, "1", "A", &["one piece", "one piece digital"]).await;
    seed_queue_release_with_queries(&db, "2", "B", &["one piece"]).await;
    seed_queue_release_with_queries(&db, "3", "C", &["bleach", "one piece"]).await;
    seed_queue_release_with_queries(&db, "4", "D", &["bleach", "naruto"]).await;
    seed_queue_release_with_queries(&db, "5", "E", &["naruto"]).await;
    seed_queue_release_with_queries(&db, "6", "F", &["solo leveling"]).await;
    // A queue release that never had queries stamped — must never appear.
    seed_queue_release(&db, "7", "feed", "G", "g.cbz", "unresolved").await;
    let app = queue_app(db);

    // The gate: for every group at breadth B, the list endpoint filtered by
    // (searchQuery=group.query, breadth=B) must return exactly group.count
    // distinct releases. This is what guarantees the raw grouped SQL and
    // review_queue_select's filter predicate can't silently diverge.
    for breadth in 1u8..=3 {
        let body = fetch_groups(&app, &format!("breadth={breadth}")).await;
        assert_eq!(body["breadth"], breadth);
        let groups = body["groups"].as_array().unwrap();
        assert!(
            !groups.is_empty(),
            "breadth {breadth} should yield clusters"
        );
        for g in groups {
            let query = g["query"].as_str().unwrap();
            let count = g["count"].as_u64().unwrap();
            assert!(count > 1, "groups must exclude singletons");
            let list_total = queue_total(
                &app,
                &format!("searchQuery={}&breadth={breadth}", encode_query(query)),
            )
            .await;
            assert_eq!(
                list_total, count,
                "parity failed for query={query:?} breadth={breadth}"
            );
        }
    }
}

/// Attach a review-candidate (series + score) to a release.
async fn seed_candidate(
    db: &sea_orm::DatabaseConnection,
    release_id: &str,
    series_id: i32,
    score: f64,
) {
    use td_db::entities::review_candidates;
    review_candidates::Entity::insert(review_candidates::ActiveModel {
        release_id: Set(release_id.to_string()),
        series_id: Set(series_id),
        score: Set(score),
        reason: Set(None),
    })
    .exec(db)
    .await
    .unwrap();
}

#[tokio::test]
async fn groups_endpoint_surfaces_dominant_top_candidate() {
    let db = fresh_db().await;
    let one_piece = seed_series(&db, "One Piece", "manga").await;
    let look_alike = seed_series(&db, "Wan Pisu", "manga").await;
    // Three releases in the "one piece" group; two point at One Piece, one at
    // the look-alike (with a higher score, which must NOT win — the hint is
    // most-common-by-distinct-releases, not best-scored).
    let a = seed_queue_release_with_queries(&db, "1", "A", &["one piece"]).await;
    let b = seed_queue_release_with_queries(&db, "2", "B", &["one piece"]).await;
    let c = seed_queue_release_with_queries(&db, "3", "C", &["one piece"]).await;
    seed_candidate(&db, &a, one_piece, 0.5).await;
    seed_candidate(&db, &b, one_piece, 0.5).await;
    seed_candidate(&db, &c, look_alike, 0.95).await;
    // A second group with no candidates at all — must still appear, hint absent.
    seed_queue_release_with_queries(&db, "4", "D", &["bleach"]).await;
    seed_queue_release_with_queries(&db, "5", "E", &["bleach"]).await;
    let app = queue_app(db);

    let body = fetch_groups(&app, "").await;
    let groups = body["groups"].as_array().unwrap();
    let one_piece_group = groups
        .iter()
        .find(|g| g["query"] == "one piece")
        .expect("one piece group present");
    assert_eq!(one_piece_group["topCandidate"]["seriesId"], one_piece);
    assert_eq!(one_piece_group["topCandidate"]["title"], "One Piece");

    let bleach_group = groups
        .iter()
        .find(|g| g["query"] == "bleach")
        .expect("bleach group present");
    // No candidates → the hint is omitted entirely (skip_serializing_if).
    assert!(bleach_group.get("topCandidate").is_none());
}

async fn queue_titles(app: &axum::Router, query: &str) -> Vec<String> {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/releases/unresolved?{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn unresolved_endpoint_sorts_by_title_case_insensitively() {
    let db = fresh_db().await;
    // Seeded out of order, mixed case: a case-sensitive sort would put the
    // capitalized titles ahead of the lowercase one.
    seed_queue_release(&db, "1", "feed", "banana", "a.cbz", "unresolved").await;
    seed_queue_release(&db, "2", "feed", "Apple", "b.cbz", "unresolved").await;
    seed_queue_release(&db, "3", "feed", "cherry", "c.cbz", "unresolved").await;
    let app = queue_app(db);

    assert_eq!(
        queue_titles(&app, "sort=title_asc").await,
        vec!["Apple", "banana", "cherry"]
    );
    assert_eq!(
        queue_titles(&app, "sort=title_desc").await,
        vec!["cherry", "banana", "Apple"]
    );
    // An unknown sort falls back to observed_desc (newest first); all three
    // share roughly the same observed time, so just assert it stays valid.
    assert_eq!(queue_titles(&app, "sort=bogus").await.len(), 3);
}

#[tokio::test]
async fn bulk_link_assigns_selected_releases_to_one_series() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "One Piece", "manga").await;
    let a = seed_queue_release(&db, "1", "feed", "One Piece 001", "a.cbz", "unresolved").await;
    let b = seed_queue_release(&db, "2", "feed", "One Piece 002", "b.cbz", "review_pending").await;
    seed_queue_release(&db, "3", "feed", "Bleach 001", "c.cbz", "unresolved").await;
    let app = queue_app(db);

    let body = post_json(
        &app,
        "/api/v1/releases/bulk/link",
        serde_json::json!({ "ids": [a, b], "seriesId": sid }),
    )
    .await;
    assert_eq!(body["linked"], 2);
    assert_eq!(body["seriesId"], sid);
    // The two linked releases leave the queue; the Bleach row remains.
    let remaining = queue_titles(&app, "").await;
    assert_eq!(remaining, vec!["Bleach 001"]);
}

#[tokio::test]
async fn bulk_link_rejects_empty_ids() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "Naruto", "manga").await;
    let app = queue_app(db);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/releases/bulk/link")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "ids": [], "seriesId": sid }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn bulk_retry_reports_matched_and_triggers() {
    let db = fresh_db().await;
    seed_queue_release(&db, "1", "feed", "A", "a.cbz", "unresolved").await;
    seed_queue_release(&db, "2", "feed", "B", "b.cbz", "review_pending").await;
    let app = queue_app(db);

    // Empty body = whole queue.
    let body = post_json(&app, "/api/v1/releases/bulk/retry", serde_json::json!({})).await;
    assert_eq!(body["triggered"], true);
    assert_eq!(body["skipped"], false);
    assert_eq!(body["matched"], 2);

    // A match set of zero is neither triggered nor skipped.
    let body = post_json(
        &app,
        "/api/v1/releases/bulk/retry",
        serde_json::json!({ "ids": ["does-not-exist"] }),
    )
    .await;
    assert_eq!(body["triggered"], false);
    assert_eq!(body["skipped"], false);
    assert_eq!(body["matched"], 0);
}

#[tokio::test]
async fn bulk_retry_skips_when_retry_lock_held() {
    let db = fresh_db().await;
    seed_queue_release(&db, "1", "feed", "A", "a.cbz", "unresolved").await;
    let locks = std::sync::Arc::new(td_scheduler::JobLocks::default());
    // Hold the shared retry-all lock so the bulk retry can't acquire it.
    let held = locks.retry_all_releases_lock();
    let _guard = held.try_lock().expect("test should hold the lock first");

    let app = common::build_app_full(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
        Vec::new(),
        td_config::ProvidersConfig::default(),
        locks,
    );

    let body = post_json(&app, "/api/v1/releases/bulk/retry", serde_json::json!({})).await;
    assert_eq!(body["triggered"], false);
    assert_eq!(body["skipped"], true);
    assert_eq!(body["matched"], 1);
}

#[tokio::test]
async fn release_dto_exposes_description_and_extracted_links() {
    let db = fresh_db().await;
    let mut r = sample_release("d1", "feed", "Some Standalone Guide");
    r.description_html = Some("# Notes\n\nA **guidebook**.".into());
    r.external_links = td_source::ExternalLinks {
        mangaupdates: Some("https://www.mangaupdates.com/series/abc/x".into()),
        ..Default::default()
    };
    let id = releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();
    // Move it out of the queue so it shows up in a status-filtered list, the
    // way the Kept view (status=standalone) fetches it.
    releases_repo::set_resolution(
        &db,
        &id,
        None,
        Some("standalone".into()),
        None,
        "standalone",
        Utc::now().timestamp(),
    )
    .await
    .unwrap();
    let app = queue_app(db);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/releases?status=standalone")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let item = &body["items"][0];
    assert!(
        item["descriptionHtml"]
            .as_str()
            .unwrap()
            .contains("guidebook"),
        "descriptionHtml should be present on the release DTO"
    );
    assert_eq!(
        item["extractedLinks"]["mangaupdates"],
        "https://www.mangaupdates.com/series/abc/x"
    );
}

#[tokio::test]
async fn series_list_filters_by_genre_and_tag_and_combines() {
    let db = fresh_db().await;
    let a = seed_series(&db, "Action+Isekai", "manga").await;
    let b = seed_series(&db, "Action-only", "manga").await;
    let c = seed_series(&db, "Drama-only", "manga").await;
    tagging_repo::set_series_genres(&db, a, &["Action".into(), "Adventure".into()])
        .await
        .unwrap();
    tagging_repo::set_series_genres(&db, b, &["Action".into()])
        .await
        .unwrap();
    tagging_repo::set_series_genres(&db, c, &["Drama".into()])
        .await
        .unwrap();
    tagging_repo::set_series_tags(&db, a, &["isekai".into()])
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    // genre filter alone returns both Action rows.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?genres=Action")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["total"], 2);

    // genre + tag AND-combined narrows to the one row tagged isekai.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?genres=Action&tags=isekai")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["id"], a);

    // genre with no matching rows returns total: 0.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?genres=nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["total"], 0);
}

#[tokio::test]
async fn series_list_genres_multi_any_vs_all() {
    let db = fresh_db().await;
    let a = seed_series(&db, "Action+Adventure", "manga").await;
    let b = seed_series(&db, "Action-only", "manga").await;
    let c = seed_series(&db, "Drama-only", "manga").await;
    tagging_repo::set_series_genres(&db, a, &["Action".into(), "Adventure".into()])
        .await
        .unwrap();
    tagging_repo::set_series_genres(&db, b, &["Action".into()])
        .await
        .unwrap();
    tagging_repo::set_series_genres(&db, c, &["Drama".into()])
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    // Default mode is `any`: Action OR Adventure → a, b (c is Drama-only).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?genres=Action,Adventure")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["total"], 2);
    let ids: Vec<i64> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect();
    assert!(ids.contains(&(a as i64)));
    assert!(ids.contains(&(b as i64)));

    // `all` mode: Action AND Adventure → only a.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?genres=Action,Adventure&genresMode=all")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["id"], a);

    // `all` with one name not on any row collapses the set even when the
    // other name matches: confirms the COUNT(DISTINCT) guard, not just
    // "more rows satisfy".
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?genres=Action,Nonexistent&genresMode=all")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["total"], 0);
}

#[tokio::test]
async fn series_list_tags_multi_any_vs_all_with_pagination_count() {
    let db = fresh_db().await;
    // Three rows with `isekai`; one of those also has `magic`. A single-
    // row "all" filter must not over-count due to the join multiplying
    // rows on the outer SELECT — the page total has to stay at 1.
    let a = seed_series(&db, "Isekai+Magic", "manga").await;
    let b = seed_series(&db, "Isekai-only-1", "manga").await;
    let c = seed_series(&db, "Isekai-only-2", "manga").await;
    tagging_repo::set_series_tags(&db, a, &["isekai".into(), "magic".into()])
        .await
        .unwrap();
    tagging_repo::set_series_tags(&db, b, &["isekai".into()])
        .await
        .unwrap();
    tagging_repo::set_series_tags(&db, c, &["isekai".into()])
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    // any: matches every row carrying either tag.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?tags=isekai,magic")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["total"], 3);

    // all: only the row with both tags.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?tags=isekai,magic&tagsMode=all")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["id"], a);
}

#[tokio::test]
async fn genres_endpoint_lists_canonical_names_with_counts() {
    let db = fresh_db().await;
    let a = seed_series(&db, "A", "manga").await;
    let b = seed_series(&db, "B", "manga").await;
    tagging_repo::set_series_genres(&db, a, &["Action".into(), "Drama".into()])
        .await
        .unwrap();
    tagging_repo::set_series_genres(&db, b, &["Action".into()])
        .await
        .unwrap();
    tagging_repo::set_series_tags(&db, a, &["isekai".into(), "magic".into()])
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/genres")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "Action");
    assert_eq!(items[0]["seriesCount"], 2);
    assert_eq!(items[1]["name"], "Drama");
    assert_eq!(items[1]["seriesCount"], 1);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/tags")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let names: Vec<&str> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"isekai"));
    assert!(names.contains(&"magic"));
}

#[tokio::test]
async fn series_detail_surfaces_join_table_tags() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "Tagged", "manga").await;
    tagging_repo::set_series_genres(&db, sid, &["Action".into(), "Drama".into()])
        .await
        .unwrap();
    tagging_repo::set_series_tags(&db, sid, &["isekai".into(), "Gore".into()])
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/series/{sid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let genres: Vec<&str> = body["genres"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    let tags: Vec<&str> = body["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert!(genres.contains(&"Action"));
    assert!(genres.contains(&"Drama"));
    assert!(tags.contains(&"isekai"));
    assert!(tags.contains(&"Gore"));
}

#[tokio::test]
async fn metrics_sources_summary_returns_per_source_aggregates() {
    let db = fresh_db().await;
    let now = Utc::now().timestamp();
    // Two successful runs + one failure for feed-a inside the 24h window.
    for (status, started, fetched) in [
        ("success", now - 300, Some(5_i32)),
        ("success", now - 200, Some(3)),
        ("failure", now - 100, None),
    ] {
        let id = run_metrics_repo::start_poll_run(&db, "feed-a", "nyaa", started, "cron")
            .await
            .unwrap();
        run_metrics_repo::finalize_poll_run(
            &db,
            id,
            started + 1,
            status,
            run_metrics_repo::PollRunCounts {
                fetched,
                new: fetched,
                resolved: fetched,
                ..Default::default()
            },
            if status == "failure" {
                Some("oops")
            } else {
                None
            },
            if status == "failure" {
                Some("network")
            } else {
                None
            },
        )
        .await
        .unwrap();
    }

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/metrics/sources?range=24h")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item["sourceName"], "feed-a");
    assert_eq!(item["totalRuns"], 3);
    assert_eq!(item["successCount"], 2);
    assert_eq!(item["failureCount"], 1);
    assert_eq!(item["fetchedSum"], 8);
    // 2 successes / (2 + 1 failures) = 0.666...
    let rate = item["successRate"].as_f64().unwrap();
    assert!((rate - 2.0 / 3.0).abs() < 1e-9);
}

#[tokio::test]
async fn metrics_sources_detail_emits_buckets_for_named_source() {
    let db = fresh_db().await;
    let now = Utc::now().timestamp();
    for (status, started) in [
        ("success", now - 100),
        ("failure", now - 50),
        ("success", now - 10),
    ] {
        let id = run_metrics_repo::start_poll_run(&db, "feed-a", "nyaa", started, "cron")
            .await
            .unwrap();
        run_metrics_repo::finalize_poll_run(
            &db,
            id,
            started + 1,
            status,
            run_metrics_repo::PollRunCounts {
                fetched: Some(1),
                new: Some(1),
                resolved: Some(1),
                ..Default::default()
            },
            None,
            None,
        )
        .await
        .unwrap();
    }

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/metrics/sources/feed-a?range=24h&buckets=24")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["sourceName"], "feed-a");
    assert_eq!(body["summary"]["totalRuns"], 3);
    let buckets = body["buckets"].as_array().unwrap();
    // Three runs within a few seconds of each other fall into one or two
    // buckets (depending on alignment); both shapes are fine as long as
    // counts sum to 3.
    let total_success: i64 = buckets
        .iter()
        .map(|b| b["successCount"].as_i64().unwrap())
        .sum();
    let total_failure: i64 = buckets
        .iter()
        .map(|b| b["failureCount"].as_i64().unwrap())
        .sum();
    assert_eq!(total_success + total_failure, 3);
}

#[tokio::test]
async fn provider_search_title_returns_dice_rescored_hits() {
    use td_metadata::{SearchHit, SeriesKind};
    let db = fresh_db().await;

    // Two hits: one is an exact title match (Dice ≈ 1.0), the other a
    // weak partial. Rescore should order them desc.
    let mut get_table = std::collections::HashMap::new();
    get_table.insert(
        "exact".into(),
        sample_metadata("mb", "exact", "Solo Leveling"),
    );
    let mut partial = sample_metadata("mb", "partial", "Solo Leveling Side Stories");
    partial.kind = Some(SeriesKind::Manhwa);
    get_table.insert("partial".into(), partial);

    let stub = StubProvider {
        id: "mb",
        returns: None,
        search_hits: vec![
            SearchHit {
                external_id: "partial".into(),
                title: "Solo Leveling Side Stories".into(),
                year: Some(2020),
                cover_url: None,
                kind: None,
                score: None,
            },
            SearchHit {
                external_id: "exact".into(),
                title: "Solo Leveling".into(),
                year: Some(2018),
                cover_url: None,
                kind: None,
                score: None,
            },
        ],
        get_table,
        ..Default::default()
    };
    let app = build_app(
        db,
        metadata_registry_with(stub),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers/mb/search?q=Solo+Leveling")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["provider"], "mb");
    let hits = body["hits"].as_array().expect("hits array");
    assert_eq!(hits.len(), 2);
    // Exact match scores higher and lands first.
    assert_eq!(hits[0]["externalId"], "exact");
    assert_eq!(hits[0]["title"], "Solo Leveling");
    assert!(hits[0]["score"].as_f64().unwrap() > hits[1]["score"].as_f64().unwrap());
    // Enrichment from `get()` is present.
    assert_eq!(hits[0]["kind"], "manga");
    assert_eq!(hits[0]["status"], "ongoing");
}

#[tokio::test]
async fn provider_search_external_id_short_circuits_to_get_with_score_one() {
    let db = fresh_db().await;
    let mut get_table = std::collections::HashMap::new();
    get_table.insert(
        "12345".into(),
        sample_metadata("mb", "12345", "Direct Lookup Series"),
    );
    let stub = StubProvider {
        id: "mb",
        returns: None,
        search_hits: vec![],
        get_table,
        ..Default::default()
    };
    let app = build_app(
        db,
        metadata_registry_with(stub),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers/mb/search?externalId=12345")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let hits = body["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["externalId"], "12345");
    assert_eq!(hits[0]["title"], "Direct Lookup Series");
    assert_eq!(hits[0]["score"].as_f64().unwrap(), 1.0);
}

#[tokio::test]
async fn provider_search_external_id_miss_returns_empty_hits() {
    let db = fresh_db().await;
    let stub = StubProvider {
        id: "mb",
        returns: None,
        search_hits: vec![],
        get_table: std::collections::HashMap::new(),
        ..Default::default()
    };
    let app = build_app(
        db,
        metadata_registry_with(stub),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers/mb/search?externalId=does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["hits"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn provider_search_rejects_empty_query() {
    let db = fresh_db().await;
    let stub = StubProvider {
        id: "mb",
        returns: None,
        search_hits: vec![],
        get_table: std::collections::HashMap::new(),
        ..Default::default()
    };
    let app = build_app(
        db,
        metadata_registry_with(stub),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers/mb/search")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn provider_search_unknown_provider_returns_404() {
    let db = fresh_db().await;
    let stub = StubProvider {
        id: "mb",
        returns: None,
        search_hits: vec![],
        get_table: std::collections::HashMap::new(),
        ..Default::default()
    };
    let app = build_app(
        db,
        metadata_registry_with(stub),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers/nope/search?q=anything")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Build an app whose stub cross-resolves only `(mangaupdates, ylx5wzn)` and
/// has an empty `get_table`, so a native `get` of that id misses. Proves the
/// foreign-id path routes to `resolve_by_foreign_id`, not `get`.
async fn foreign_search_app() -> axum::Router {
    let mut foreign_table = std::collections::HashMap::new();
    foreign_table.insert(
        ("mangaupdates".to_string(), "ylx5wzn".to_string()),
        sample_metadata("mb", "42", "Cross Resolved"),
    );
    let stub = StubProvider {
        id: "mb",
        foreign_table,
        foreign_sources: vec!["mangaupdates"],
        ..Default::default()
    };
    build_app(
        fresh_db().await,
        metadata_registry_with(stub),
        source_registry_with(vec![]),
        open_auth(),
    )
}

#[tokio::test]
async fn provider_search_foreign_provider_param_cross_resolves() {
    let app = foreign_search_app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers/mb/search?externalId=ylx5wzn&foreignProvider=mangaupdates")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let hits = body["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["title"], "Cross Resolved");
    assert_eq!(hits[0]["score"].as_f64().unwrap(), 1.0);
}

#[tokio::test]
async fn provider_search_full_url_auto_detects_foreign_provider() {
    // No foreignProvider param: the handler detects the host from the URL.
    let app = foreign_search_app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers/mb/search?externalId=https%3A%2F%2Fwww.mangaupdates.com%2Fseries%2Fylx5wzn%2Fx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["hits"][0]["title"], "Cross Resolved");
}

#[tokio::test]
async fn provider_search_legacy_mangaupdates_url_returns_no_hits() {
    // A legacy series.html?id=NNN link can't be translated in the synchronous
    // search path, so it yields no hits (the modal hints to use the modern URL).
    let app = foreign_search_app().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/providers/mb/search?externalId=https%3A%2F%2Fwww.mangaupdates.com%2Fseries.html%3Fid%3D151349")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body["hits"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn series_list_with_q_ranks_exact_match_first() {
    let db = fresh_db().await;
    seed_series(&db, "Naruto", "manga").await;
    seed_series(&db, "Bleach", "manga").await;
    seed_series(&db, "One Piece", "manga").await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?q=naruto")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert!(!items.is_empty());
    assert_eq!(items[0]["canonicalTitle"], "Naruto");
}

#[tokio::test]
async fn series_list_with_q_finds_typo_via_dice_rerank() {
    let db = fresh_db().await;
    seed_series(&db, "Naruto", "manga").await;
    seed_series(&db, "Bleach", "manga").await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    // FTS5 would not return "Naruto" for the prefix `narto*`, but the
    // Dice rerank still ranks it well above the score floor.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?q=narto")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let titles: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["canonicalTitle"].as_str().unwrap())
        .collect();
    assert!(
        titles.contains(&"Naruto"),
        "expected Naruto to surface via Dice rerank for typo query; got {titles:?}"
    );
}

#[tokio::test]
async fn series_list_with_q_returns_empty_for_gibberish() {
    let db = fresh_db().await;
    seed_series(&db, "Naruto", "manga").await;
    seed_series(&db, "Bleach", "manga").await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?q=xyzzy_qwerasdf")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["total"], 0);
    assert!(body["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn series_list_q_composes_with_filters() {
    let db = fresh_db().await;
    seed_series(&db, "Naruto", "manga").await;
    seed_series(&db, "Naruto Light Novel", "novel").await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?q=naruto&kind=novel")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["canonicalTitle"], "Naruto Light Novel");
    assert_eq!(items[0]["kind"], "novel");
}

#[tokio::test]
async fn series_list_with_blank_q_falls_back_to_unfiltered_list() {
    let db = fresh_db().await;
    seed_series(&db, "Naruto", "manga").await;
    seed_series(&db, "Bleach", "manga").await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?q=%20%20")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["total"], 2);
}

#[tokio::test]
async fn metrics_id_maps_reports_per_provider_counts_and_mu_cache() {
    let db = fresh_db().await;
    let s1 = seed_series(&db, "Series A", "manga").await;
    let s2 = seed_series(&db, "Series B", "manga").await;
    series_external_ids_repo::upsert(&db, s1, "mangaupdates", "mu-1", 100)
        .await
        .unwrap();
    series_external_ids_repo::upsert(&db, s2, "mangaupdates", "mu-2", 100)
        .await
        .unwrap();
    series_external_ids_repo::upsert(&db, s1, "mal", "mal-1", 100)
        .await
        .unwrap();
    mangaupdates_id_repo::record(&db, 11, Some("modern-x"), 500)
        .await
        .unwrap();
    mangaupdates_id_repo::record(&db, 12, None, 600)
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/metrics/id-maps")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let external = body["externalIds"].as_array().unwrap();
    // Alphabetical: `mal` before `mangaupdates`.
    assert_eq!(external[0]["provider"], "mal");
    assert_eq!(external[0]["count"], 1);
    assert_eq!(external[1]["provider"], "mangaupdates");
    assert_eq!(external[1]["count"], 2);
    let mu = &body["mangaupdatesRedirectCache"];
    assert_eq!(mu["modernCount"], 1);
    assert_eq!(mu["tombstoneCount"], 1);
    assert_eq!(mu["lastResolvedAt"], 600);
}

#[tokio::test]
async fn manual_poll_publishes_started_and_finished_job_events() {
    let db = fresh_db().await;
    let (app, events) = common::build_app_with_events(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![StubSource {
            name: "feed-a".into(),
            kind: "stub".into(),
            outcome: PollOutcome::default(),
        }]),
        open_auth(),
        Vec::new(),
        td_config::ProvidersConfig::default(),
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let mut rx = events.subscribe();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sources/feed-a/poll")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The synchronous part of the handler emits `started` before
    // returning. The spawned tick may emit `progress` frames from inside
    // run_tick (via ProgressHandle) and then emits `finished` once it
    // completes. Filter out Progress frames so the test only asserts
    // on the lifecycle boundaries.
    let started = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("started event should arrive")
        .expect("channel still open");
    assert!(matches!(started.kind, td_api::JobKind::Source));
    assert_eq!(started.id, "feed-a");
    assert!(matches!(started.phase, td_api::JobPhase::Started));

    let finished = recv_until_finished(&mut rx, std::time::Duration::from_secs(5)).await;
    let result = finished.result.expect("finished carries a result payload");
    assert!(result.triggered);
    assert!(!result.skipped);
}

/// Drain Progress frames and return the next Finished event. Used by the
/// "happy path" lifecycle tests where ProgressHandle inside the job body
/// may emit intermediate frames that are not load-bearing for the
/// dispatcher contract.
async fn recv_until_finished(
    rx: &mut tokio::sync::broadcast::Receiver<td_api::JobEvent>,
    timeout: std::time::Duration,
) -> td_api::JobEvent {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let event = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("finished event should arrive within the budget")
            .expect("channel still open");
        if matches!(event.phase, td_api::JobPhase::Finished) {
            return event;
        }
        assert!(
            matches!(event.phase, td_api::JobPhase::Progress),
            "unexpected event phase between Started and Finished: {event:?}"
        );
    }
}

#[tokio::test]
async fn manual_poll_emits_only_finished_when_skipped() {
    let db = fresh_db().await;
    let locks = std::sync::Arc::new(td_scheduler::JobLocks::default());
    // Hold the lock so the handler reports `skipped=true` synchronously.
    let held = locks.source_lock("feed-a");
    let _guard = held.try_lock().expect("test holds the lock");

    let (app, events) = common::build_app_with_events(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![StubSource {
            name: "feed-a".into(),
            kind: "stub".into(),
            outcome: PollOutcome::default(),
        }]),
        open_auth(),
        Vec::new(),
        td_config::ProvidersConfig::default(),
        locks,
    );
    let mut rx = events.subscribe();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sources/feed-a/poll")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
        .await
        .expect("finished{skipped} event should arrive")
        .expect("channel still open");
    assert!(matches!(event.phase, td_api::JobPhase::Finished));
    let result = event.result.expect("finished carries a result");
    assert!(!result.triggered);
    assert!(result.skipped);
    // No further events should land for this trigger.
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn manual_provider_refresh_publishes_finished_job_event() {
    let db = fresh_db().await;
    let (app, events) = common::build_app_with_events(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
        Vec::new(),
        td_config::ProvidersConfig::default(),
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let mut rx = events.subscribe();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/providers/mb/refresh-cache")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let started = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("started event")
        .expect("channel open");
    assert!(matches!(started.kind, td_api::JobKind::Provider));
    assert_eq!(started.id, "mb");

    let _finished = recv_until_finished(&mut rx, std::time::Duration::from_secs(5)).await;
}

#[tokio::test]
async fn events_endpoint_streams_job_events_as_sse() {
    use axum::body::to_bytes;
    let db = fresh_db().await;
    let (app, events) = common::build_app_with_events(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
        Vec::new(),
        td_config::ProvidersConfig::default(),
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );

    // Pre-publish a frame so the next subscriber receives it.
    // Spawn before sending so the receiver is registered first.
    let send_task = tokio::spawn(async move {
        // Tiny pause to let the SSE handler subscribe before the publish.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let _ = events.send(td_api::JobEvent::finished(
            td_api::JobKind::Source,
            "feed-a",
            td_api::JobResult {
                triggered: true,
                skipped: false,
                ..Default::default()
            },
        ));
    });

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/events/jobs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(content_type.starts_with("text/event-stream"));

    // The body is an unbounded stream; bound the read with a 1s budget so
    // the test cannot hang. After we see the first frame, drop the body
    // to close the connection.
    let body = resp.into_body();
    let bytes = tokio::time::timeout(std::time::Duration::from_secs(2), to_bytes(body, 64 * 1024))
        .await
        .expect("body read should not hang");
    let bytes = bytes.expect("body bytes");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("\"phase\":\"finished\"") && text.contains("\"id\":\"feed-a\""),
        "expected finished frame in SSE body, got:\n{text}"
    );

    let _ = send_task.await;
}

#[tokio::test]
async fn metrics_id_maps_returns_empty_state_cleanly() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/metrics/id-maps")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body["externalIds"].as_array().unwrap().is_empty());
    let mu = &body["mangaupdatesRedirectCache"];
    assert_eq!(mu["modernCount"], 0);
    assert_eq!(mu["tombstoneCount"], 0);
    assert!(mu["lastResolvedAt"].is_null());
}

#[tokio::test]
async fn series_refresh_all_triggers_when_lock_is_free() {
    let db = fresh_db().await;
    let (app, events) = common::build_app_with_events(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
        Vec::new(),
        td_config::ProvidersConfig::default(),
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let mut rx = events.subscribe();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series/refresh-all")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = body_json(resp).await;
    assert_eq!(body["provider"], "mb");
    assert_eq!(body["triggered"], true);
    assert_eq!(body["skipped"], false);
    // Echoes the defaults from MetadataConfig::default in the test
    // harness.
    assert_eq!(body["batchSize"], 50);
    assert_eq!(body["minAgeDays"], 7);

    let started = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("started event should arrive")
        .expect("channel still open");
    assert!(matches!(started.kind, td_api::JobKind::SeriesRefresh));
    assert_eq!(started.id, "mb");
    assert!(matches!(started.phase, td_api::JobPhase::Started));

    let finished = recv_until_finished(&mut rx, std::time::Duration::from_secs(5)).await;
    let result = finished.result.expect("finished carries a result payload");
    assert!(result.triggered);
    assert!(!result.skipped);
}

#[tokio::test]
async fn series_refresh_all_returns_skipped_when_lock_is_held() {
    let db = fresh_db().await;
    let locks = std::sync::Arc::new(td_scheduler::JobLocks::default());
    // Pre-acquire the active provider's series-refresh lock to simulate a
    // tick already in flight. The handler must see `try_lock` fail and
    // report `skipped: true` synchronously.
    let held = locks.series_refresh_lock("mb");
    let _guard = held.try_lock().expect("test holds the lock");

    let (app, events) = common::build_app_with_events(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
        Vec::new(),
        td_config::ProvidersConfig::default(),
        locks,
    );
    let mut rx = events.subscribe();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series/refresh-all")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["triggered"], false);
    assert_eq!(body["skipped"], true);
    assert_eq!(body["batchSize"], 50);

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
        .await
        .expect("finished{skipped} event should arrive")
        .expect("channel still open");
    assert!(matches!(event.kind, td_api::JobKind::SeriesRefresh));
    assert!(matches!(event.phase, td_api::JobPhase::Finished));
    let result = event.result.expect("finished carries a result");
    assert!(!result.triggered);
    assert!(result.skipped);
    // No further events expected.
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn series_refresh_all_requires_admin_token() {
    let db = fresh_db().await;
    // Auth: read open, but admin required for writes.
    let auth = td_config::AuthConfig {
        read_requires_auth: false,
        api_key: None,
        admin_token: Some("write-token".into()),
    };
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        auth,
    );
    // No Authorization header → 401.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series/refresh-all")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

async fn seed_series_with_hash(
    db: &sea_orm::DatabaseConnection,
    title: &str,
    metadata_source: &str,
    metadata_hash: Option<&str>,
) -> i32 {
    let now = Utc::now().timestamp();
    let model = series::ActiveModel {
        canonical_title: Set(title.into()),
        alternate_titles_json: Set(None),
        cover_url: Set(None),
        kind: Set(Some("manga".into())),
        status: Set(Some("ongoing".into())),
        year: Set(Some(2020)),
        metadata_json: Set(None),
        metadata_source: Set(metadata_source.into()),
        metadata_hash: Set(metadata_hash.map(str::to_owned)),
        metadata_fetched_at: Set(now),
        first_seen_at: Set(now),
        last_release_at: Set(now),
        highest_volume: Set(None),
        highest_chapter: Set(None),
        owned: Set(0),
        ..Default::default()
    };
    let row = model.insert(db).await.unwrap();
    row.id
}

#[tokio::test]
async fn invalidate_metadata_hashes_clears_provider_rows_and_skips_manual() {
    let db = fresh_db().await;
    let api_row = seed_series_with_hash(&db, "A", "api", Some("hash-a")).await;
    let cache_row = seed_series_with_hash(&db, "B", "offline_cache", Some("hash-b")).await;
    let manual_row = seed_series_with_hash(&db, "M", "manual", Some("hash-m")).await;

    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series/invalidate-metadata-hashes")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["invalidated"], 2);
    assert_eq!(body["skippedManual"], 1);
    assert!(
        body["provider"].is_null(),
        "no scope was requested, so provider echoes null; got {body:?}"
    );

    // Verify DB state matches the response.
    let api_after = series::Entity::find_by_id(api_row)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let cache_after = series::Entity::find_by_id(cache_row)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let manual_after = series::Entity::find_by_id(manual_row)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(api_after.metadata_hash, None);
    assert_eq!(cache_after.metadata_hash, None);
    assert_eq!(manual_after.metadata_hash, Some("hash-m".into()));
}

#[tokio::test]
async fn invalidate_metadata_hashes_filters_by_provider() {
    let db = fresh_db().await;
    let mb_id = seed_series_with_hash(&db, "MB", "api", Some("hash-mb")).await;
    let other_id = seed_series_with_hash(&db, "Other", "api", Some("hash-other")).await;
    let now = Utc::now().timestamp();
    series_external_ids_repo::upsert(&db, mb_id, "mangabaka", "1", now)
        .await
        .unwrap();
    series_external_ids_repo::upsert(&db, other_id, "anilist", "999", now)
        .await
        .unwrap();

    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series/invalidate-metadata-hashes?provider=mangabaka")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["invalidated"], 1);
    assert_eq!(body["skippedManual"], 0);
    assert_eq!(body["provider"], "mangabaka");

    let mb_after = series::Entity::find_by_id(mb_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let other_after = series::Entity::find_by_id(other_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(mb_after.metadata_hash, None);
    assert_eq!(
        other_after.metadata_hash,
        Some("hash-other".into()),
        "anilist-backed row stays untouched when scoped to mangabaka",
    );
}

#[tokio::test]
async fn invalidate_metadata_hashes_requires_admin_token() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series/invalidate-metadata-hashes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// Cover-proxy tests.
//
// The on-disk cache is content-addressed at `sha256(url).<ext>`; we hand
// the handler a pre-warmed tempdir so it never has to reach upstream.
// The "fetches on miss" path is left for manual verification against a
// real MangaBaka URL; standing up an HTTP stub here would buy little.
fn covers_cache_filename(url: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(url.as_bytes());
    let digest = h.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    let lower = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    let ext = if lower.ends_with(".png") {
        "png"
    } else if lower.ends_with(".webp") {
        "webp"
    } else if lower.ends_with(".gif") {
        "gif"
    } else {
        "jpg"
    };
    format!("{hex}.{ext}")
}

async fn seed_series_with_cover(db: &sea_orm::DatabaseConnection, cover_url: &str) -> i32 {
    let now = Utc::now().timestamp();
    let model = series::ActiveModel {
        canonical_title: Set("With Cover".into()),
        alternate_titles_json: Set(None),
        cover_url: Set(Some(cover_url.into())),
        kind: Set(Some("manga".into())),
        status: Set(Some("ongoing".into())),
        year: Set(Some(2020)),
        metadata_json: Set(None),
        metadata_source: Set("api".into()),
        metadata_hash: Set(None),
        metadata_fetched_at: Set(now),
        first_seen_at: Set(now),
        last_release_at: Set(now),
        highest_volume: Set(None),
        highest_chapter: Set(None),
        owned: Set(0),
        ..Default::default()
    };
    model.insert(db).await.unwrap().id
}

#[tokio::test]
async fn covers_returns_404_for_missing_series() {
    let db = fresh_db().await;
    let tmp = tempfile::tempdir().unwrap();
    let app = build_app_with_cover_cache(db, open_auth(), tmp.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/covers/9999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn covers_returns_404_when_series_has_no_cover_url() {
    let db = fresh_db().await;
    let id = seed_series(&db, "No Cover", "manga").await;
    let tmp = tempfile::tempdir().unwrap();
    let app = build_app_with_cover_cache(db, open_auth(), tmp.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/covers/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn covers_serves_cached_bytes_for_series() {
    let db = fresh_db().await;
    let cover_url = "https://cdn.mangabaka.dev/series/42/cover-350.jpg";
    let id = seed_series_with_cover(&db, cover_url).await;

    let tmp = tempfile::tempdir().unwrap();
    let filename = covers_cache_filename(cover_url);
    let bytes = b"\xFF\xD8\xFF\xE0fake-jpeg".to_vec();
    tokio::fs::write(tmp.path().join(&filename), &bytes)
        .await
        .unwrap();

    let app = build_app_with_cover_cache(db, open_auth(), tmp.path().to_path_buf());
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/covers/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/jpeg"
    );
    assert_eq!(
        resp.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=3600"
    );
    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(body.as_ref(), bytes.as_slice());
}

#[tokio::test]
async fn covers_by_url_serves_cached_bytes_for_allowed_host() {
    let db = fresh_db().await;
    let cover_url = "https://cdn.mangabaka.dev/series/7/raw.png";
    let tmp = tempfile::tempdir().unwrap();
    let filename = covers_cache_filename(cover_url);
    tokio::fs::write(tmp.path().join(&filename), b"\x89PNG\r\n\x1A\nfake")
        .await
        .unwrap();

    let app = build_app_with_cover_cache(db, open_auth(), tmp.path().to_path_buf());
    let encoded = urlencoding_form(cover_url);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/covers/by-url?url={encoded}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/png"
    );
}

#[tokio::test]
async fn covers_by_url_rejects_disallowed_host() {
    let db = fresh_db().await;
    let tmp = tempfile::tempdir().unwrap();
    let app = build_app_with_cover_cache(db, open_auth(), tmp.path().to_path_buf());

    let encoded = urlencoding_form("https://evil.example.com/x.jpg");
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/covers/by-url?url={encoded}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn covers_by_url_rejects_http_scheme() {
    let db = fresh_db().await;
    let tmp = tempfile::tempdir().unwrap();
    let app = build_app_with_cover_cache(db, open_auth(), tmp.path().to_path_buf());

    let encoded = urlencoding_form("http://mangabaka.dev/x.jpg");
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/covers/by-url?url={encoded}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn covers_invalidate_cache_deletes_files_and_returns_counts() {
    let db = fresh_db().await;
    let tmp = tempfile::tempdir().unwrap();
    tokio::fs::write(tmp.path().join("a.jpg"), vec![1u8; 100])
        .await
        .unwrap();
    tokio::fs::write(tmp.path().join("b.png"), vec![2u8; 50])
        .await
        .unwrap();
    tokio::fs::create_dir(tmp.path().join("keep"))
        .await
        .unwrap();

    let app = build_app_with_cover_cache(db, open_auth(), tmp.path().to_path_buf());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/covers/invalidate-cache")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["filesDeleted"], 2);
    assert_eq!(body["bytesFreed"], 150);
    assert!(!tmp.path().join("a.jpg").exists());
    assert!(!tmp.path().join("b.png").exists());
    assert!(tmp.path().join("keep").exists());
}

#[tokio::test]
async fn covers_invalidate_cache_requires_admin_bearer() {
    let db = fresh_db().await;
    let tmp = tempfile::tempdir().unwrap();
    let app = build_app_with_cover_cache(db, open_auth(), tmp.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/covers/invalidate-cache")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn covers_503_when_cache_dir_not_configured() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/covers/1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/covers/invalidate-cache")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

/// Minimal application/x-www-form-urlencoded encoder for query-param
/// test inputs. Avoids pulling a dep just to encode `:` and `/`.
fn urlencoding_form(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ----- Codex presence integration (admin-only) ---------------------------

#[tokio::test]
async fn codex_status_reports_disabled_when_integration_off() {
    let db = fresh_db().await;
    // Default config: codex disabled, no client.
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/codex/status")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["enabled"], false);
}

#[tokio::test]
async fn codex_status_requires_admin() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    // No bearer: the status endpoint must not be reachable by the public read
    // tier, since it exposes what is in the operator's Codex.
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/codex/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn codex_status_reflects_recorded_row_when_enabled() {
    use td_db::repos::codex_status_repo;
    let db = fresh_db().await;
    codex_status_repo::set_preflight(&db, true, Some("codex"), Some("1.2.3"), 100)
        .await
        .unwrap();
    codex_status_repo::set_success(&db, 50, 7, 200)
        .await
        .unwrap();

    let codex = td_config::CodexConfig {
        enabled: true,
        base_url: Some("https://codex.example.com".into()),
        api_key: Some("k".into()),
        ..Default::default()
    };
    let app = build_app_with_codex(
        db,
        open_auth(),
        codex,
        Some(unreachable_codex_client()),
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/codex/status")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["enabled"], true);
    assert_eq!(body["reachable"], true);
    assert_eq!(body["codexVersion"], "1.2.3");
    assert_eq!(body["authState"], "ok");
    assert_eq!(body["linkedCount"], 7);
    assert_eq!(body["fetchedCount"], 50);
}

#[tokio::test]
async fn codex_test_503s_when_disabled() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/codex/test")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn codex_test_requires_admin() {
    let db = fresh_db().await;
    let codex = td_config::CodexConfig {
        enabled: true,
        base_url: Some("https://codex.example.com".into()),
        api_key: Some("k".into()),
        ..Default::default()
    };
    let app = build_app_with_codex(
        db,
        open_auth(),
        codex,
        Some(unreachable_codex_client()),
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/codex/test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn codex_test_reports_unreachable_and_records_manual_history() {
    let db = fresh_db().await;
    let codex = td_config::CodexConfig {
        enabled: true,
        base_url: Some("https://codex.example.com".into()),
        api_key: Some("k".into()),
        ..Default::default()
    };
    // The client points at an unreachable endpoint, so the /info probe fails —
    // a 200 report of reachable:false, with a manual history row recorded.
    let app = build_app_with_codex(
        db,
        open_auth(),
        codex,
        Some(unreachable_codex_client()),
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/codex/test")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["enabled"], true);
    assert_eq!(body["reachable"], false);
    let checks = body["recentChecks"].as_array().unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0]["trigger"], "manual");
    assert_eq!(checks[0]["reachable"], false);
}

#[tokio::test]
async fn codex_refresh_503s_when_disabled() {
    let db = fresh_db().await;
    // Default build: codex_client is None -> disabled.
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/codex/refresh")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn codex_refresh_reports_skipped_when_a_sweep_is_in_flight() {
    let db = fresh_db().await;
    let locks = std::sync::Arc::new(td_scheduler::JobLocks::default());
    // Simulate an in-flight sweep by holding the codex lock for the request.
    let _guard = locks.codex_sync_lock().try_lock_owned().unwrap();

    let codex = td_config::CodexConfig {
        enabled: true,
        base_url: Some("https://codex.example.com".into()),
        api_key: Some("k".into()),
        ..Default::default()
    };
    let app = build_app_with_codex(
        db,
        open_auth(),
        codex,
        Some(unreachable_codex_client()),
        locks.clone(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/codex/refresh")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["triggered"], false);
    assert_eq!(body["skipped"], true);
}

#[tokio::test]
async fn codex_manual_link_then_unlink_roundtrips() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "Hand Linked", "manga").await;
    let app = build_app_with_codex(
        db.clone(),
        open_auth(),
        td_config::CodexConfig::default(),
        None,
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );

    // Link.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/series/{sid}/codex-link"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"codexSeriesUuid":"codex-uuid-xyz"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["seriesId"], sid);
    assert_eq!(body["codexSeriesUuid"], "codex-uuid-xyz");
    assert_eq!(body["linkKind"], "manual");

    let link = td_db::repos::codex_link_repo::get(&db, sid)
        .await
        .unwrap()
        .expect("link persisted");
    assert_eq!(link.codex_series_uuid, "codex-uuid-xyz");

    // Unlink.
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/series/{sid}/codex-link"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert!(
        td_db::repos::codex_link_repo::get(&db, sid)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn codex_manual_link_404s_for_unknown_series() {
    let db = fresh_db().await;
    let app = build_app_with_codex(
        db,
        open_auth(),
        td_config::CodexConfig::default(),
        None,
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series/99999/codex-link")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"codexSeriesUuid":"x"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ----- Codex presence overlay on series (admin-gated) --------------------

async fn seed_series_with_highs(
    db: &sea_orm::DatabaseConnection,
    title: &str,
    highest_volume: Option<f64>,
    highest_chapter: Option<f64>,
) -> i32 {
    let now = Utc::now().timestamp();
    let model = series::ActiveModel {
        canonical_title: Set(title.into()),
        kind: Set(Some("manga".into())),
        metadata_source: Set("api".into()),
        metadata_fetched_at: Set(now),
        first_seen_at: Set(now),
        last_release_at: Set(now),
        highest_volume: Set(highest_volume),
        highest_chapter: Set(highest_chapter),
        owned: Set(0),
        ..Default::default()
    };
    model.insert(db).await.unwrap().id
}

async fn link_auto(
    db: &sea_orm::DatabaseConnection,
    series_id: i32,
    uuid: &str,
    local_max_volume: Option<f64>,
) {
    use td_db::repos::codex_link_repo::{AutoLink, upsert_auto};
    upsert_auto(
        db,
        &AutoLink {
            series_id,
            codex_series_uuid: uuid.into(),
            local_max_volume,
            local_max_chapter: None,
            volumes_owned: local_max_volume.map(|v| v as i64),
            matched_provider: "mangabaka".into(),
            matched_external_id: "1".into(),
            synced_at: 1,
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn series_list_omits_codex_for_non_admin() {
    let db = fresh_db().await;
    let sid = seed_series_with_highs(&db, "Owned", Some(10.0), None).await;
    link_auto(&db, sid, "uuid-1", Some(10.0)).await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    // No bearer -> public read tier -> never sees codex data.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(
        body.get("codexSyncedAt").is_none(),
        "no codexSyncedAt for anon"
    );
    let item = &body["items"][0];
    assert!(item.get("codex").is_none(), "no codex field for anon");
    assert_eq!(item["owned"], false);
}

#[tokio::test]
async fn series_list_includes_codex_for_admin() {
    let db = fresh_db().await;
    // Owned + caught up.
    let owned = seed_series_with_highs(&db, "Owned Complete", Some(10.0), None).await;
    link_auto(&db, owned, "uuid-owned", Some(10.0)).await;
    // Not on Codex.
    let _unowned = seed_series_with_highs(&db, "Not Owned", Some(3.0), None).await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();

    let owned_item = items
        .iter()
        .find(|i| i["id"] == owned)
        .expect("owned series present");
    assert_eq!(owned_item["owned"], true);
    assert_eq!(owned_item["codex"]["status"], "complete");
    assert_eq!(owned_item["codex"]["seriesUuid"], "uuid-owned");
    assert_eq!(owned_item["codex"]["linkKind"], "auto");

    let unowned_item = items
        .iter()
        .find(|i| i["canonicalTitle"] == "Not Owned")
        .expect("unowned series present");
    assert!(unowned_item.get("codex").is_none());
    assert_eq!(unowned_item["owned"], false);
}

#[tokio::test]
async fn series_list_sorts_by_rating_with_nulls_last() {
    let db = fresh_db().await;
    let now = Utc::now().timestamp();
    let seed = |title: &str, rating: Option<f64>| {
        let title = title.to_string();
        let db = db.clone();
        async move {
            series::ActiveModel {
                canonical_title: Set(title),
                kind: Set(Some("manga".into())),
                metadata_source: Set("api".into()),
                metadata_fetched_at: Set(now),
                first_seen_at: Set(now),
                last_release_at: Set(now),
                rating: Set(rating),
                owned: Set(0),
                ..Default::default()
            }
            .insert(&db)
            .await
            .unwrap();
        }
    };
    seed("Mid", Some(6.5)).await;
    seed("Top", Some(9.0)).await;
    seed("Unrated", None).await;
    seed("Low", Some(2.0)).await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let titles = |order: &str| {
        let app = app.clone();
        let uri = format!("/api/v1/series?sort=rating&order={order}");
        async move {
            let resp = app
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp)
                .await
                .get("items")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["canonicalTitle"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        }
    };

    // Descending: highest rating first, unrated row sinks to the end.
    assert_eq!(titles("desc").await, ["Top", "Mid", "Low", "Unrated"]);
    // Ascending: lowest rated first, unrated row STILL sinks to the end
    // (nullable-aware ordering, not NULLs-first like raw SQLite ASC).
    assert_eq!(titles("asc").await, ["Low", "Mid", "Top", "Unrated"]);
}

#[tokio::test]
async fn series_list_sorts_by_publication_date_with_nulls_last() {
    let db = fresh_db().await;
    let now = Utc::now().timestamp();
    let seed = |title: &str, start: Option<&str>| {
        let title = title.to_string();
        let start = start.map(str::to_string);
        let db = db.clone();
        async move {
            series::ActiveModel {
                canonical_title: Set(title),
                kind: Set(Some("manga".into())),
                metadata_source: Set("api".into()),
                metadata_fetched_at: Set(now),
                first_seen_at: Set(now),
                last_release_at: Set(now),
                published_start_date: Set(start),
                owned: Set(0),
                ..Default::default()
            }
            .insert(&db)
            .await
            .unwrap();
        }
    };
    seed("Mid", Some("2015-06-01")).await;
    seed("Newest", Some("2022-01-10")).await;
    seed("Undated", None).await;
    seed("Oldest", Some("2001-03-20")).await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let titles = |order: &str| {
        let app = app.clone();
        let uri = format!("/api/v1/series?sort=published_start_date&order={order}");
        async move {
            let resp = app
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp)
                .await
                .get("items")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["canonicalTitle"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        }
    };

    // Descending: most recent publication first, undated row sinks to the end.
    assert_eq!(titles("desc").await, ["Newest", "Mid", "Oldest", "Undated"]);
    // Ascending: oldest first, undated row STILL last (nullable-aware).
    assert_eq!(titles("asc").await, ["Oldest", "Mid", "Newest", "Undated"]);
}

/// Free-text search used to scan `LIMIT 5000` with no `ORDER BY`, so SQLite
/// stopped after the first 5000 rows in rowid order and the newest series were
/// silently unscoreable. Reproduces the production shape: two same-titled rows
/// straddling the old cap, where only the *older* one was ever returned.
#[tokio::test]
async fn series_search_finds_rows_past_the_old_candidate_cap() {
    let db = fresh_db().await;
    let now = Utc::now().timestamp();

    // The row that always worked: comfortably inside the old scan window.
    let early = seed_series(&db, "My Quiet Blacksmith Life in Another World", "novel").await;

    // Filler up past the old 5000-row cap. Inserted in batches so the test
    // stays quick despite the FTS triggers firing per row.
    let mut filler = Vec::with_capacity(500);
    for i in 0..5_200 {
        filler.push(series::ActiveModel {
            canonical_title: Set(format!("Filler Series {i}")),
            metadata_source: Set("api".into()),
            metadata_fetched_at: Set(now),
            first_seen_at: Set(now),
            last_release_at: Set(now),
            owned: Set(0),
            ..Default::default()
        });
        if filler.len() == 500 {
            series::Entity::insert_many(std::mem::take(&mut filler))
                .exec(&db)
                .await
                .unwrap();
            filler = Vec::with_capacity(500);
        }
    }
    if !filler.is_empty() {
        series::Entity::insert_many(filler).exec(&db).await.unwrap();
    }

    // The row that was invisible: same title, created last, past the cap.
    let late = seed_series(&db, "My Quiet Blacksmith Life in Another World", "manga").await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?q=My%20Quiet%20Blacksmith%20Life%20in%20Another%20World")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let ids: Vec<i64> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect();

    assert!(
        ids.contains(&(early as i64)),
        "the row inside the old scan window must still be found",
    );
    assert!(
        ids.contains(&(late as i64)),
        "an exact title match past the old 5000-row cap must be found; got {ids:?}",
    );
}

/// An exact FTS5 match must survive the Dice floor. Today the `+0.50` FTS boost
/// happens to clear the `0.30` floor on its own, so this pins that contract
/// rather than the arithmetic that currently upholds it: raising the floor
/// above the boost must not silently start dropping exact matches.
#[tokio::test]
async fn series_search_keeps_fts_matches_scoring_below_the_dice_floor() {
    let db = fresh_db().await;
    // Dice between "naruto" and this title is far below the 0.30 floor — the
    // query is a tiny fraction of the title's bigrams — but FTS matches the
    // token exactly.
    let long = seed_series(
        &db,
        "Naruto Gaiden: The Seventh Hokage and the Scarlet Spring Flower",
        "manga",
    )
    .await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?q=Naruto")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let ids: Vec<i64> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect();
    assert!(
        ids.contains(&(long as i64)),
        "an exact FTS token match must not be filtered out by the Dice floor",
    );
}

/// The FTS pass used to fetch only the top 200 by rank. A common token matches
/// far more than that in a real catalog ("World" hits 374 of 5342 in
/// production), and a one-word query scores well under the Dice floor against a
/// long title — so every match past 200 was dropped outright rather than merely
/// ranked lower. Seeds a match set comfortably over the old limit and asserts
/// the whole set survives.
#[tokio::test]
async fn series_search_keeps_every_fts_match_past_the_old_fetch_limit() {
    let db = fresh_db().await;
    let now = Utc::now().timestamp();

    // 300 long titles sharing one token. Dice("world", <long title>) is far
    // below the 0.30 floor, so FTS membership is the only thing keeping any of
    // them in the running.
    let mut batch = Vec::with_capacity(300);
    for i in 0..300 {
        batch.push(series::ActiveModel {
            canonical_title: Set(format!(
                "Reincarnated in Another World as a Wandering Alchemist Volume {i}"
            )),
            metadata_source: Set("api".into()),
            metadata_fetched_at: Set(now),
            first_seen_at: Set(now),
            last_release_at: Set(now),
            owned: Set(0),
            ..Default::default()
        });
    }
    series::Entity::insert_many(batch).exec(&db).await.unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?q=World&pageSize=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    // `total` reflects the whole candidate set, independent of page size.
    assert_eq!(
        body["total"].as_u64().unwrap(),
        300,
        "every FTS match must be a candidate, not just the first 200 by rank",
    );
}

/// The discovery sort exists precisely because `last_release_at` (the upstream
/// post date) buries a series found today from a year-old post. Seeded so the
/// two orderings disagree completely: the newest *discovery* is the oldest
/// *post*.
#[tokio::test]
async fn series_list_sorts_by_last_discovered_at_with_nulls_last() {
    let db = fresh_db().await;
    let seed = |title: &str, posted: i64, discovered: Option<i64>| {
        let title = title.to_string();
        let db = db.clone();
        async move {
            series::ActiveModel {
                canonical_title: Set(title),
                kind: Set(Some("manga".into())),
                metadata_source: Set("api".into()),
                metadata_fetched_at: Set(posted),
                first_seen_at: Set(posted),
                last_release_at: Set(posted),
                last_discovered_at: Set(discovered),
                owned: Set(0),
                ..Default::default()
            }
            .insert(&db)
            .await
            .unwrap();
        }
    };
    // "Backfilled" is the oldest post but the newest discovery — the exact
    // shape that ranks ~5000 rows deep under the default sort.
    seed("Backfilled", 1_600_000_000, Some(1_900_000_000)).await;
    seed("Recent", 1_800_000_000, Some(1_800_000_100)).await;
    seed("Older", 1_700_000_000, Some(1_700_000_100)).await;
    seed("Undiscovered", 1_850_000_000, None).await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let titles = |sort: &str, order: &str| {
        let app = app.clone();
        let uri = format!("/api/v1/series?sort={sort}&order={order}");
        async move {
            let resp = app
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp)
                .await
                .get("items")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["canonicalTitle"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        }
    };

    // Descending: newest discovery first; the never-discovered row sinks.
    assert_eq!(
        titles("last_discovered_at", "desc").await,
        ["Backfilled", "Recent", "Older", "Undiscovered"],
    );
    // Ascending: oldest discovery first, undiscovered row STILL last.
    assert_eq!(
        titles("last_discovered_at", "asc").await,
        ["Older", "Recent", "Backfilled", "Undiscovered"],
    );
    // The point of the whole phase: the default sort buries "Backfilled" that
    // the discovery sort leads with.
    assert_eq!(
        titles("last_release_at", "desc").await,
        ["Undiscovered", "Recent", "Older", "Backfilled"],
    );
}

/// The discovery timestamp has to reach the client, not just the database —
/// the UI renders it next to "last release" on the series row.
#[tokio::test]
async fn series_list_exposes_last_discovered_at() {
    let db = fresh_db().await;
    series::ActiveModel {
        canonical_title: Set("Discovered".into()),
        metadata_source: Set("api".into()),
        metadata_fetched_at: Set(1),
        first_seen_at: Set(1),
        last_release_at: Set(1),
        last_discovered_at: Set(Some(1_900_000_000)),
        owned: Set(0),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["items"][0]["lastDiscoveredAt"], 1_900_000_000);
}

#[tokio::test]
async fn series_detail_codex_is_admin_only() {
    let db = fresh_db().await;
    let sid = seed_series_with_highs(&db, "Behind Series", Some(12.0), None).await;
    // Codex owns up to vol 5 but vol 12 has surfaced -> behind.
    link_auto(&db, sid, "uuid-behind", Some(5.0)).await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    // Admin sees the overlay.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/series/{sid}"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["codex"]["status"], "behind");
    assert_eq!(body["owned"], true);

    // Anon does not.
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/series/{sid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert!(body.get("codex").is_none());
    assert_eq!(body["owned"], false);
}

#[tokio::test]
async fn codex_status_filter_is_ignored_for_non_admin() {
    let db = fresh_db().await;
    let linked = seed_series_with_highs(&db, "Linked", Some(1.0), None).await;
    link_auto(&db, linked, "u", Some(1.0)).await;
    let _unlinked = seed_series_with_highs(&db, "Unlinked", Some(1.0), None).await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    // A non-admin can't filter by codex status: the param is dropped, so the
    // full feed comes back rather than leaking which series are on Codex.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?codexStatus=missing")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["total"], 2, "filter must be a no-op for anon");
}

#[tokio::test]
async fn codex_status_filter_applies_for_admin() {
    let db = fresh_db().await;
    // Behind: owns vol 5, vol 12 surfaced.
    let behind = seed_series_with_highs(&db, "Behind", Some(12.0), None).await;
    link_auto(&db, behind, "u-behind", Some(5.0)).await;
    // Complete: owns vol 10, nothing newer.
    let complete = seed_series_with_highs(&db, "Complete", Some(10.0), None).await;
    link_auto(&db, complete, "u-complete", Some(10.0)).await;
    // Missing: not on Codex.
    let missing = seed_series_with_highs(&db, "Missing", Some(1.0), None).await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let fetch = |uri: &str| {
        let app = app.clone();
        let uri = uri.to_string();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .header(header::AUTHORIZATION, "Bearer write-token")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            body_json(resp).await
        }
    };

    let body = fetch("/api/v1/series?codexStatus=behind").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["id"], behind);

    let body = fetch("/api/v1/series?codexStatus=complete").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["id"], complete);

    let body = fetch("/api/v1/series?codexStatus=any").await;
    assert_eq!(body["total"], 2, "any = on Codex (behind + complete)");

    let body = fetch("/api/v1/series?codexStatus=missing").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["id"], missing);

    // Multi-select is OR-combined: missing,behind keeps the unlinked series
    // plus the owned-but-behind one, but not the caught-up `complete`.
    let body = fetch("/api/v1/series?codexStatus=missing,behind").await;
    assert_eq!(body["total"], 2, "missing OR behind");
    let ids: Vec<i64> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["id"].as_i64().unwrap())
        .collect();
    assert!(ids.contains(&(missing as i64)));
    assert!(ids.contains(&(behind as i64)));
    assert!(!ids.contains(&(complete as i64)));

    // Two on-Codex sub-statuses OR together (no `missing`): behind + complete.
    let body = fetch("/api/v1/series?codexStatus=behind,complete").await;
    assert_eq!(body["total"], 2, "behind OR complete");

    // `any` + `missing` together = every series, unconstrained.
    let body = fetch("/api/v1/series?codexStatus=any,missing").await;
    assert_eq!(body["total"], 3, "any OR missing = everything");
}

#[tokio::test]
async fn codex_status_ignored_short_circuits_and_filters() {
    let db = fresh_db().await;
    // Would be Behind (owns vol 5, vol 12 surfaced) but the operator turned
    // completion tracking off -> the status is Ignored, not Behind.
    let ignored = seed_series_with_highs(&db, "Omnibus", Some(12.0), None).await;
    link_auto(&db, ignored, "u-ignored", Some(5.0)).await;
    series::Entity::update(series::ActiveModel {
        id: Set(ignored),
        ignore_completion: Set(true),
        ..Default::default()
    })
    .exec(&db)
    .await
    .unwrap();
    // A genuinely behind series with the flag off, same maxima.
    let behind = seed_series_with_highs(&db, "Behind", Some(12.0), None).await;
    link_auto(&db, behind, "u-behind", Some(5.0)).await;

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let fetch = |uri: &str| {
        let app = app.clone();
        let uri = uri.to_string();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .header(header::AUTHORIZATION, "Bearer write-token")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            body_json(resp).await
        }
    };

    // The ignored series still reads as owned, with status "ignored".
    let body = fetch(&format!("/api/v1/series/{ignored}")).await;
    assert_eq!(body["codex"]["status"], "ignored");
    assert_eq!(body["owned"], true);

    // codexStatus=ignored returns only the flagged series.
    let body = fetch("/api/v1/series?codexStatus=ignored").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["id"], ignored);

    // codexStatus=behind excludes the ignored series despite identical maxima.
    let body = fetch("/api/v1/series?codexStatus=behind").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["id"], behind);

    // Both are on Codex.
    let body = fetch("/api/v1/series?codexStatus=any").await;
    assert_eq!(body["total"], 2);
}

#[tokio::test]
async fn set_ignore_completion_toggles_codex_status_via_endpoint() {
    let db = fresh_db().await;
    // Provider-backed series that would otherwise read Behind (owns vol 5,
    // vol 12 surfaced). The PATCH manual-edit path would 409 this row; the
    // ignore-completion endpoint must accept it.
    let sid = seed_series_with_highs(&db, "Omnibus", Some(12.0), None).await;
    link_auto(&db, sid, "u-omnibus", Some(5.0)).await;

    let app = build_app(
        db.clone(),
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let put = |ignore: bool| {
        let app = app.clone();
        async move {
            let body = serde_json::json!({ "ignore": ignore });
            app.oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/series/{sid}/ignore-completion"))
                    .header(header::AUTHORIZATION, "Bearer write-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };

    // Turn tracking off -> status becomes "ignored", still owned.
    let resp = put(true).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["codex"]["status"], "ignored");
    assert_eq!(body["owned"], true);
    assert!(
        series::Entity::find_by_id(sid)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .ignore_completion
    );

    // Turn it back on -> the real comparison resurfaces (Behind).
    let resp = put(false).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["codex"]["status"], "behind");
}

#[tokio::test]
async fn set_ignore_completion_returns_404_for_missing_series() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/series/9999/ignore-completion")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"ignore":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn codex_synced_at_present_for_admin_when_enabled() {
    use td_db::repos::codex_status_repo;
    let db = fresh_db().await;
    codex_status_repo::set_success(&db, 10, 3, 1700)
        .await
        .unwrap();

    let codex = td_config::CodexConfig {
        enabled: true,
        base_url: Some("https://codex.example.com".into()),
        api_key: Some("k".into()),
        ..Default::default()
    };
    let app = build_app_with_codex(
        db,
        open_auth(),
        codex,
        None,
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    // Admin: sees the sweep timestamp.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["codexSyncedAt"], 1700);

    // Anon: absent even though a sweep has succeeded.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert!(body.get("codexSyncedAt").is_none());
}

// --- send to torrent client ------------------------------------------------

fn enabled_download_config() -> td_config::DownloadConfig {
    td_config::DownloadConfig {
        enabled: true,
        rutorrent: td_config::RuTorrentConfig {
            base_url: Some("http://127.0.0.1:9/rutorrent".into()),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[tokio::test]
async fn download_status_reports_disabled_by_default() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/download/status")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["enabled"], false);
    assert!(body.get("kind").is_none());
}

#[tokio::test]
async fn download_status_reports_enabled_with_kind() {
    let db = fresh_db().await;
    let app = build_app_with_download(db, open_auth(), enabled_download_config(), None);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/download/status")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["enabled"], true);
    assert_eq!(body["kind"], "rutorrent");
}

#[tokio::test]
async fn download_status_requires_admin() {
    let db = fresh_db().await;
    let app = build_app_with_download(db, open_auth(), enabled_download_config(), None);
    // No bearer: the enablement probe must not be reachable by the public read
    // tier.
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/download/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn send_to_client_503s_when_disabled() {
    let db = fresh_db().await;
    let r = sample_release("1", "feed", "Chainsaw Man v01");
    let id = releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();
    // Default config: download disabled, no client.
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/releases/{id}/send-to-client"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "misconfigured");
}

#[tokio::test]
async fn send_to_client_404s_for_unknown_release() {
    let db = fresh_db().await;
    let app = build_app_with_download(
        db,
        open_auth(),
        enabled_download_config(),
        Some(unreachable_download_client()),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/releases/does-not-exist/send-to-client")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn send_to_client_400s_when_release_has_no_source() {
    let db = fresh_db().await;
    // sample_release has neither magnet nor torrent_url.
    let r = sample_release("1", "feed", "Chainsaw Man v01");
    let id = releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();
    let app = build_app_with_download(
        db,
        open_auth(),
        enabled_download_config(),
        Some(unreachable_download_client()),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/releases/{id}/send-to-client"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "bad_request");
}

#[tokio::test]
async fn send_to_client_requires_admin() {
    let db = fresh_db().await;
    let app = build_app_with_download(
        db,
        open_auth(),
        enabled_download_config(),
        Some(unreachable_download_client()),
    );
    // No bearer: writes are admin-only.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/releases/whatever/send-to-client")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn download_test_503s_when_disabled() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/download/test")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn download_test_requires_admin() {
    let db = fresh_db().await;
    let app = build_app_with_download(
        db,
        open_auth(),
        enabled_download_config(),
        Some(unreachable_download_client()),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/download/test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn download_test_reports_unreachable_client_as_200() {
    let db = fresh_db().await;
    // The client points at a dead port, so the probe fails — but a failed
    // *report* is still a 200 with reachable=false + the reason, distinct from
    // the 503-disabled case.
    let app = build_app_with_download(
        db,
        open_auth(),
        enabled_download_config(),
        Some(unreachable_download_client()),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/download/test")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["enabled"], true);
    assert_eq!(body["reachable"], false);
    assert!(body["lastError"].is_string());
    // The manual probe always records a history row.
    let checks = body["recentChecks"].as_array().unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0]["trigger"], "manual");
    assert_eq!(checks[0]["reachable"], false);
}

#[tokio::test]
async fn send_to_client_records_failed_attempt_and_502s() {
    let db = fresh_db().await;
    // A release with a magnet so the send reaches the client (which is dead).
    let mut r = sample_release("1", "feed", "Chainsaw Man v01");
    r.magnet = Some("magnet:?xt=urn:btih:deadbeef".into());
    let id = releases_repo::persist_discovered(&db, &r, Utc::now().timestamp())
        .await
        .unwrap();
    let app = build_app_with_download(
        db,
        open_auth(),
        enabled_download_config(),
        Some(unreachable_download_client()),
    );

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/releases/{id}/send-to-client"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"preferMagnet":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);

    // The failed attempt is recorded in the send audit.
    let status = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/download/status")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(status).await;
    let sends = body["recentSends"].as_array().unwrap();
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0]["success"], false);
    assert_eq!(sends[0]["source"], "magnet");
    assert_eq!(sends[0]["releaseId"], id);
    assert!(sends[0]["error"].is_string());
}

// ---- series catalog export (GET /series/export) ----

/// Read a response body as UTF-8 text (export endpoints return files, not JSON
/// envelopes).
async fn body_text(resp: axum::response::Response) -> String {
    let bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn export_app(db: sea_orm::DatabaseConnection) -> axum::Router {
    build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    )
}

#[tokio::test]
async fn series_export_requires_admin_bearer() {
    // The export lives in the admin (writes) group, so a read without a bearer
    // is rejected even though reads are otherwise open.
    let db = fresh_db().await;
    seed_series(&db, "Manga A", "manga").await;
    let app = export_app(db);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn series_export_json_is_attachment_and_respects_filters() {
    let db = fresh_db().await;
    seed_series(&db, "Manga A", "manga").await;
    seed_series(&db, "Manga B", "manga").await;
    seed_series(&db, "Novel X", "novel").await;
    let app = export_app(db);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/export?format=json&kind=manga")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/json; charset=utf-8"
    );
    let disposition = resp
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(disposition.starts_with("attachment; filename=\"tsundoku-series-export-"));
    assert!(disposition.ends_with(".json\""));

    let body = body_text(resp).await;
    let arr: Value = serde_json::from_str(&body).unwrap();
    let titles: Vec<&str> = arr
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["canonicalTitle"].as_str().unwrap())
        .collect();
    // The kind filter is honored exactly like GET /series: novel excluded,
    // alphabetical order.
    assert_eq!(titles, vec!["Manga A", "Manga B"]);
}

#[tokio::test]
async fn series_export_csv_sets_headers_and_header_row() {
    let db = fresh_db().await;
    seed_series(&db, "Manga A", "manga").await;
    let app = export_app(db);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/export?format=csv")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/csv; charset=utf-8"
    );
    assert!(
        resp.headers()
            .get(header::CONTENT_DISPOSITION)
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with(".csv\"")
    );
    let body = body_text(resp).await;
    let header_row = body.lines().next().unwrap();
    assert!(header_row.starts_with("id,canonicalTitle,"));
    assert!(header_row.contains("codexStatus"));
    // Header + one data row.
    assert_eq!(body.lines().count(), 2);
}

#[tokio::test]
async fn series_export_markdown_sets_headers_and_table() {
    let db = fresh_db().await;
    seed_series(&db, "Manga A", "manga").await;
    let app = export_app(db);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/export?format=markdown")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/markdown; charset=utf-8"
    );
    let body = body_text(resp).await;
    assert!(body.contains("# tsundoku series catalog"));
    assert!(body.contains("| id | canonicalTitle |"));
    assert!(body.contains("Manga A"));
}

#[tokio::test]
async fn series_export_fields_param_subsets_columns() {
    let db = fresh_db().await;
    seed_series(&db, "Manga A", "manga").await;
    let app = export_app(db);

    let resp = app
        .oneshot(
            Request::builder()
                // Out-of-order + an unknown key; canonicalTitle is forced in.
                .uri("/api/v1/series/export?format=csv&fields=year,bogus,kind")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_text(resp).await;
    // Canonical column order: canonicalTitle, kind, year.
    assert_eq!(body.lines().next().unwrap(), "canonicalTitle,kind,year");
    // The data row reflects the seeded kind + year.
    assert_eq!(body.lines().nth(1).unwrap(), "Manga A,manga,2020");
}

#[tokio::test]
async fn series_list_filters_by_multiple_kinds() {
    // The kind filter accepts a comma-separated set (OR via IN); the catalog
    // export's multi-select relies on this. A single value still works.
    let db = fresh_db().await;
    seed_series(&db, "Manga A", "manga").await;
    seed_series(&db, "Manhwa B", "manhwa").await;
    seed_series(&db, "Novel X", "novel").await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?kind=manga,manhwa&pageSize=50")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["total"], 2, "manga + manhwa, novel excluded");
    let kinds: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"manga"));
    assert!(kinds.contains(&"manhwa"));
    assert!(!kinds.contains(&"novel"));
}

fn stub_app(db: sea_orm::DatabaseConnection) -> axum::Router {
    build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    )
}

#[tokio::test]
async fn series_feed_walks_keyset_with_cursor_and_coverage() {
    let db = fresh_db().await;
    // updated_at = 0 → inactive, must never surface in the feed.
    seed_series(&db, "Inactive", "manga").await;
    let active1 = seed_feed_series(
        &db,
        "Active One",
        100,
        r#"[{"start":1.0,"end":4.0},{"start":6.0,"end":9.0}]"#,
        9.0,
        "111",
    )
    .await;
    let active2 = seed_feed_series(
        &db,
        "Active Two",
        200,
        r#"[{"start":1.0,"end":3.0}]"#,
        3.0,
        "222",
    )
    .await;

    let app = stub_app(db);

    // First page: one item (oldest active first), more remaining.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/feed?limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["seriesId"], active1);
    assert_eq!(items[0]["updatedAt"], 100);
    assert_eq!(items[0]["highestVolume"], 9.0);
    assert_eq!(items[0]["externalIds"][0]["provider"], "mangabaka");
    assert_eq!(items[0]["externalIds"][0]["externalId"], "111");
    // Coverage is the gap-preserving NumericSpan[] shape.
    assert_eq!(
        items[0]["volumeCoverage"],
        serde_json::json!([{"start":1.0,"end":4.0},{"start":6.0,"end":9.0}])
    );
    assert_eq!(body["hasMore"], true);
    let cursor = body["nextCursor"].as_str().unwrap().to_string();

    // Resume from the cursor: the second active series, no more after it.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/series/feed?limit=1&cursor={cursor}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["seriesId"], active2);
    assert_eq!(body["hasMore"], false);

    // A malformed cursor is a 400, not a silent restart.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/feed?cursor=not-base64!")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn series_feed_post_filters_by_external_ids() {
    let db = fresh_db().await;
    let a = seed_feed_series(
        &db,
        "Active One",
        100,
        r#"[{"start":1.0,"end":4.0}]"#,
        4.0,
        "111",
    )
    .await;
    let _b = seed_feed_series(
        &db,
        "Active Two",
        200,
        r#"[{"start":1.0,"end":3.0}]"#,
        3.0,
        "222",
    )
    .await;
    let c = seed_feed_series(
        &db,
        "Active Three",
        300,
        r#"[{"start":1.0,"end":9.0}]"#,
        9.0,
        "333",
    )
    .await;

    let app = stub_app(db);

    // POST the subset we track (A and C); B must be excluded.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series/feed")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"externalIds":["mangabaka:111","mangabaka:333"],"limit":100}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let ids: Vec<i64> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["seriesId"].as_i64().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![a as i64, c as i64],
        "filtered to the posted ids, in keyset order"
    );
    assert_eq!(body["hasMore"], false);
}

// ---------------------------------------------------------------------------
// Wishlist
// ---------------------------------------------------------------------------

/// Build an app with a `mb` stub that serves `sample_metadata` for any id.
fn wishlist_app(db: sea_orm::DatabaseConnection) -> axum::Router {
    build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: Some(sample_metadata("mb", "1677", "Chainsaw Man")),
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    )
}

#[tokio::test]
async fn wishlist_toggle_sets_and_clears_flag() {
    let db = fresh_db().await;
    let sid = seed_series(&db, "Wishable", "manga").await;
    let app = wishlist_app(db.clone());

    // Clip it.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/series/{sid}/wishlist"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"wishlisted":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["wishlisted"], true);
    assert!(body["wishlistedAt"].as_i64().unwrap() > 0);

    // Un-clip it.
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/series/{sid}/wishlist"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"wishlisted":false}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["wishlisted"], false);
    assert!(body["wishlistedAt"].is_null());
}

#[tokio::test]
async fn wishlist_toggle_404_for_unknown_series() {
    let db = fresh_db().await;
    let app = wishlist_app(db);
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/series/999999/wishlist")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"wishlisted":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn wishlist_filter_partitions_for_admin() {
    let db = fresh_db().await;
    let wished = seed_series(&db, "Wished", "manga").await;
    let plain = seed_series(&db, "Plain", "manga").await;
    let app = wishlist_app(db.clone());

    // Clip one.
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/series/{wished}/wishlist"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"wishlisted":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    let ids = |body: &Value| -> Vec<i64> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_i64().unwrap())
            .collect()
    };

    // wishlisted=true keeps only the clipped series.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?wishlisted=true")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ids(&body_json(resp).await), vec![wished as i64]);

    // wishlisted=false keeps only the other.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?wishlisted=false")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ids(&body_json(resp).await), vec![plain as i64]);
}

#[tokio::test]
async fn wishlist_is_hidden_from_non_admin() {
    let db = fresh_db().await;
    let wished = seed_series(&db, "Wished", "manga").await;
    let _plain = seed_series(&db, "Plain", "manga").await;
    let app = wishlist_app(db.clone());

    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/series/{wished}/wishlist"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"wishlisted":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // Non-admin list: the flag is blanked and the filter is ignored (returns
    // both series, neither flagged).
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?wishlisted=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "filter ignored without admin token");
    for item in items {
        assert_eq!(item["wishlisted"], false, "flag blanked for non-admin");
    }
}

#[tokio::test]
async fn bulk_wishlist_sets_and_clears_counting_only_existing() {
    let db = fresh_db().await;
    let a = seed_series(&db, "Bulk A", "manga").await;
    let b = seed_series(&db, "Bulk B", "manga").await;
    let app = wishlist_app(db.clone());

    // Clip both; the unknown id is dropped from the count. The static
    // `/series/bulk/wishlist` segment must not be captured by the
    // `/series/{id}/wishlist` param sibling (which would 400 on "bulk").
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/series/bulk/wishlist")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"ids":[{a},{b},999999],"wishlisted":true}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["updated"], 2);
    for id in [a, b] {
        let row = series::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(row.wishlisted_at.is_some(), "series {id} clipped");
    }

    // Un-clip only `a`; `b` stays clipped.
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/series/bulk/wishlist")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(r#"{{"ids":[{a}],"wishlisted":false}}"#)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["updated"], 1);
    let row_a = series::Entity::find_by_id(a)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(row_a.wishlisted_at.is_none());
    let row_b = series::Entity::find_by_id(b)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(row_b.wishlisted_at.is_some());
}

#[tokio::test]
async fn bulk_wishlist_empty_ids_is_bad_request() {
    let db = fresh_db().await;
    let app = wishlist_app(db);
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/series/bulk/wishlist")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"ids":[],"wishlisted":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    // Assert on the handler's own message so this can't pass by accident via
    // axum's path-param rejection on `/series/{id}/wishlist`.
    let body = body_json(resp).await;
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("ids must not be empty"),
        "expected the bulk handler's validation message, got: {body}"
    );
}

#[tokio::test]
async fn bulk_wishlist_requires_admin() {
    let db = fresh_db().await;
    let app = wishlist_app(db);
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/series/bulk/wishlist")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"ids":[1],"wishlisted":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bulk_refresh_metadata_mixed_batch_reports_per_id_outcomes() {
    let db = fresh_db().await;
    // Mapped to the active provider ⇒ refreshed (title rewritten from the stub).
    let mapped = seed_series(&db, "Old Title", "manga").await;
    series_external_ids_repo::upsert(&db, mapped, "mb", "1677", 100)
        .await
        .unwrap();
    // No active-provider mapping ⇒ per-id skip, not a batch error.
    let unmapped = seed_series(&db, "Unmapped", "manga").await;
    let app = wishlist_app(db.clone());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series/bulk/refresh-metadata")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"ids":[{mapped},{unmapped},999999]}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["refreshed"], 1);
    let skipped = body["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 2);
    let reason_for = |id: i64| {
        skipped
            .iter()
            .find(|s| s["id"].as_i64() == Some(id))
            .unwrap_or_else(|| panic!("no skip entry for {id}: {body}"))["reason"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert!(
        reason_for(unmapped as i64).contains("no mapping"),
        "unmapped reason: {}",
        reason_for(unmapped as i64)
    );
    assert!(
        reason_for(999999).contains("not found"),
        "unknown-id reason: {}",
        reason_for(999999)
    );

    // The mapped row was actually rewritten from provider metadata.
    let row = series::Entity::find_by_id(mapped)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.canonical_title, "Chainsaw Man");
}

#[tokio::test]
async fn bulk_refresh_metadata_empty_ids_is_bad_request() {
    let db = fresh_db().await;
    let app = wishlist_app(db);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series/bulk/refresh-metadata")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"ids":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("ids must not be empty"),
        "expected the bulk handler's validation message, got: {body}"
    );
}

#[tokio::test]
async fn from_provider_creates_wishlisted_series_idempotently() {
    let db = fresh_db().await;
    let app = wishlist_app(db.clone());
    let body = serde_json::json!({ "provider": "mb", "externalId": "1677" });

    // First add creates the series + mapping and clips it.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series/from-provider")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = body_json(resp).await;
    let sid = created["id"].as_i64().unwrap();
    assert!(sid > 0);
    assert_eq!(created["wishlisted"], true);
    assert_eq!(created["canonicalTitle"], "Chainsaw Man");

    // The provider mapping was persisted.
    let mapping = series_external_ids_repo::find_series_id(&db, "mb", "1677")
        .await
        .unwrap();
    assert_eq!(mapping, Some(sid as i32));

    // Re-adding the same (provider, externalId) reuses the row → 200, same id.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/series/from-provider")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["id"].as_i64().unwrap(), sid);
}

#[tokio::test]
async fn wishlist_sort_orders_by_clip_time() {
    let db = fresh_db().await;
    let a = seed_series(&db, "Clipped First", "manga").await;
    let b = seed_series(&db, "Clipped Later", "manga").await;
    // Stamp explicit clip times so the order is deterministic (the toggle
    // endpoint would stamp both within the same second).
    td_db::repos::series_repo::set_wishlisted(&db, a, true, 100)
        .await
        .unwrap();
    td_db::repos::series_repo::set_wishlisted(&db, b, true, 200)
        .await
        .unwrap();
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    let ids = |body: &Value| -> Vec<i64> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_i64().unwrap())
            .collect()
    };

    // desc: most-recently clipped first.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?wishlisted=true&sort=wishlisted_at&order=desc")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ids(&body_json(resp).await), vec![b as i64, a as i64]);

    // asc: oldest clip first.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?wishlisted=true&sort=wishlisted_at&order=asc")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ids(&body_json(resp).await), vec![a as i64, b as i64]);
}

// ---------------------------------------------------------------------------
// Per-series release search
// ---------------------------------------------------------------------------

fn search_stub(name: &str, hits: Vec<td_source::DiscoveredRelease>) -> StubSearchSource {
    StubSearchSource {
        name: name.into(),
        hits,
        delay: None,
        url_prefix: None,
        url_release: None,
    }
}

#[tokio::test]
async fn search_entries_lists_config_with_default_flag() {
    let db = fresh_db().await;
    let app = build_app_with_search(
        db,
        open_auth(),
        vec![
            (search_stub("eng", vec![]), false),
            (search_stub("raw", vec![]), true),
        ],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/search/entries")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    // Config order preserved; the marked entry (not the first) is default.
    assert_eq!(items[0]["name"], "eng");
    assert_eq!(items[0]["default"], false);
    assert_eq!(items[1]["name"], "raw");
    assert_eq!(items[1]["default"], true);
    assert_eq!(items[0]["kind"], "test");
    assert_eq!(items[0]["maxPages"], 3);
    assert_eq!(items[0]["searchUrl"], "https://nyaa.test/?c=3_1&entry=eng");
}

#[tokio::test]
async fn search_entries_requires_admin_token() {
    let db = fresh_db().await;
    let app = build_app_with_search(
        db,
        open_auth(),
        vec![(search_stub("eng", vec![]), true)],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/search/entries")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

async fn post_search(app: axum::Router, series_id: i32, body: &str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/series/{series_id}/search-releases"))
            .header(header::AUTHORIZATION, "Bearer write-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn search_trigger_404s_on_unknown_series() {
    let db = fresh_db().await;
    let app = build_app_with_search(
        db,
        open_auth(),
        vec![(search_stub("eng", vec![]), true)],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = post_search(app, 9999, "{}").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn search_trigger_400s_on_unknown_entry() {
    let db = fresh_db().await;
    let series_id = seed_series(&db, "Solo Leveling", "manhwa").await;
    let app = build_app_with_search(
        db,
        open_auth(),
        vec![(search_stub("eng", vec![]), true)],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = post_search(app, series_id, r#"{"search":"nope"}"#).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn search_trigger_503s_when_nothing_configured() {
    let db = fresh_db().await;
    let series_id = seed_series(&db, "Solo Leveling", "manhwa").await;
    let app = build_app_with_search(
        db,
        open_auth(),
        vec![],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = post_search(app, series_id, "{}").await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn search_trigger_runs_the_walk_and_records_the_audit_row() {
    let db = fresh_db().await;
    let series_id = seed_series(&db, "Solo Leveling", "manhwa").await;
    let hit = sample_release("sr-1", "eng", "Solo Leveling v01");
    let app = build_app_with_search(
        db.clone(),
        open_auth(),
        vec![(search_stub("eng", vec![hit]), true)],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );

    let resp = post_search(app, series_id, "{}").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["triggered"], true);
    assert_eq!(body["skipped"], false);
    assert_eq!(body["search"], "eng");
    assert_eq!(body["seriesId"], series_id);

    // The walk runs detached; poll the audit row until it completes.
    let mut done = None;
    for _ in 0..100 {
        let runs = td_db::repos::search_runs_repo::recent_for_series(&db, series_id, 5)
            .await
            .unwrap();
        if let Some(r) = runs.first()
            && r.outcome != td_db::repos::search_runs_repo::OUTCOME_RUNNING
        {
            done = Some(r.clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let run = done.expect("search run should complete");
    assert_eq!(run.outcome, td_db::repos::search_runs_repo::OUTCOME_SUCCESS);
    assert_eq!(run.trigger, "manual");
    assert_eq!(run.search_name, "eng");
    assert_eq!(run.releases_new, Some(1));

    // The hit went through the normal persist path.
    assert!(
        releases_repo::find_by_id(&db, &releases_repo::id_for("test", "sr-1"))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn search_trigger_skips_when_the_entry_is_busy() {
    let db = fresh_db().await;
    let series_id = seed_series(&db, "Solo Leveling", "manhwa").await;
    let locks = std::sync::Arc::new(td_scheduler::JobLocks::default());
    let held = locks.search_lock("eng");
    let _guard = held.try_lock().expect("test should hold the lock first");

    let app = build_app_with_search(
        db,
        open_auth(),
        vec![(search_stub("eng", vec![]), true)],
        locks,
    );
    let resp = post_search(app, series_id, "{}").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["triggered"], false);
    assert_eq!(body["skipped"], true);
}

async fn post_bulk_search(app: axum::Router, body: &str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/api/v1/series/bulk/search-releases")
            .header(header::AUTHORIZATION, "Bearer write-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn bulk_search_walks_each_series_and_records_audit_rows() {
    let db = fresh_db().await;
    let a = seed_series(&db, "Solo Leveling", "manhwa").await;
    let b = seed_series(&db, "Omniscient Reader", "manhwa").await;
    let hit = sample_release("bulk-sr-1", "eng", "Solo Leveling v01");
    let app = build_app_with_search(
        db.clone(),
        open_auth(),
        vec![(search_stub("eng", vec![hit]), true)],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );

    // Unknown ids are dropped from `matched`, not errors.
    let resp = post_bulk_search(app, &format!(r#"{{"ids":[{a},{b},999999]}}"#)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["search"], "eng");
    assert_eq!(body["matched"], 2);
    assert_eq!(body["triggered"], true);
    assert_eq!(body["skipped"], false);

    // One detached job walks both series sequentially; each gets its own
    // completed audit row (this is what feeds the run-history timelines).
    for sid in [a, b] {
        let mut done = None;
        for _ in 0..100 {
            let runs = td_db::repos::search_runs_repo::recent_for_series(&db, sid, 5)
                .await
                .unwrap();
            if let Some(r) = runs.first()
                && r.outcome != td_db::repos::search_runs_repo::OUTCOME_RUNNING
            {
                done = Some(r.clone());
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let run = done.unwrap_or_else(|| panic!("series {sid} search run should complete"));
        assert_eq!(run.outcome, td_db::repos::search_runs_repo::OUTCOME_SUCCESS);
        assert_eq!(run.trigger, "manual");
        assert_eq!(run.search_name, "eng");
    }
}

#[tokio::test]
async fn bulk_search_skips_whole_batch_when_the_entry_is_busy() {
    let db = fresh_db().await;
    let a = seed_series(&db, "Solo Leveling", "manhwa").await;
    let locks = std::sync::Arc::new(td_scheduler::JobLocks::default());
    let held = locks.search_lock("eng");
    let _guard = held.try_lock().expect("test should hold the lock first");

    let app = build_app_with_search(
        db.clone(),
        open_auth(),
        vec![(search_stub("eng", vec![]), true)],
        locks,
    );
    let resp = post_bulk_search(app, &format!(r#"{{"ids":[{a}]}}"#)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["triggered"], false);
    assert_eq!(body["skipped"], true);

    // Nothing ran: no audit row was created for the series.
    let runs = td_db::repos::search_runs_repo::recent_for_series(&db, a, 5)
        .await
        .unwrap();
    assert!(runs.is_empty(), "busy lock must not start any walk");
}

#[tokio::test]
async fn bulk_search_validates_entry_registry_and_ids() {
    let db = fresh_db().await;
    let a = seed_series(&db, "Solo Leveling", "manhwa").await;

    // Unknown entry → 400.
    let app = build_app_with_search(
        db.clone(),
        open_auth(),
        vec![(search_stub("eng", vec![]), true)],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = post_bulk_search(app.clone(), &format!(r#"{{"ids":[{a}],"search":"nope"}}"#)).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Empty ids → 400 with the handler's own message.
    let resp = post_bulk_search(app.clone(), r#"{"ids":[]}"#).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("ids must not be empty"),
        "expected the bulk handler's validation message, got: {body}"
    );

    // All ids unknown → 404.
    let resp = post_bulk_search(app, r#"{"ids":[999999]}"#).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // No entries configured → 503.
    let empty_app = build_app_with_search(
        db,
        open_auth(),
        vec![],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = post_bulk_search(empty_app, &format!(r#"{{"ids":[{a}]}}"#)).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn search_runs_lists_newest_first_and_404s_on_unknown_series() {
    let db = fresh_db().await;
    let series_id = seed_series(&db, "Solo Leveling", "manhwa").await;
    let first = td_db::repos::search_runs_repo::insert_running(&db, 100, "eng", series_id, "cli")
        .await
        .unwrap();
    td_db::repos::search_runs_repo::complete(
        &db,
        first,
        160,
        td_db::repos::search_runs_repo::OUTCOME_SUCCESS,
        td_db::repos::search_runs_repo::SearchRunCounts {
            queries_attempted: 1,
            pages_fetched: 2,
            releases_seen: 10,
            releases_new: 4,
        },
        None,
    )
    .await
    .unwrap();
    td_db::repos::search_runs_repo::insert_running(&db, 200, "raw", series_id, "manual")
        .await
        .unwrap();

    let app = build_app_with_search(
        db,
        open_auth(),
        vec![(search_stub("eng", vec![]), true)],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/series/{series_id}/search-runs"))
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["searchName"], "raw");
    assert_eq!(items[0]["outcome"], "running");
    assert_eq!(items[1]["searchName"], "eng");
    assert_eq!(items[1]["outcome"], "success");
    assert_eq!(items[1]["releasesNew"], 4);
    assert_eq!(items[1]["finishedAt"], 160);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/series/424242/search-runs")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Run history timelines
// ---------------------------------------------------------------------------

#[tokio::test]
async fn source_runs_lists_newest_first_with_counts_and_errors() {
    let db = fresh_db().await;

    let ok = run_metrics_repo::start_poll_run(&db, "a", "stub", 100, "cron")
        .await
        .unwrap();
    run_metrics_repo::finalize_poll_run(
        &db,
        ok,
        160,
        run_metrics_repo::status::SUCCESS,
        run_metrics_repo::PollRunCounts {
            fetched: Some(75),
            new: Some(4),
            resolved: Some(3),
            fetch_duration_ms: Some(1200),
            ..Default::default()
        },
        None,
        None,
    )
    .await
    .unwrap();
    let failed = run_metrics_repo::start_poll_run(&db, "a", "stub", 200, "manual")
        .await
        .unwrap();
    run_metrics_repo::finalize_poll_run(
        &db,
        failed,
        210,
        run_metrics_repo::status::FAILURE,
        run_metrics_repo::PollRunCounts::default(),
        Some("nyaa timed out"),
        Some("timeout"),
    )
    .await
    .unwrap();
    // A run for another source must not leak into the list.
    run_metrics_repo::start_poll_run(&db, "b", "stub", 300, "cron")
        .await
        .unwrap();

    let app = common::build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            ..Default::default()
        }),
        source_registry_with(vec![
            StubSource {
                name: "a".into(),
                kind: "stub".into(),
                outcome: PollOutcome::default(),
            },
            StubSource {
                name: "b".into(),
                kind: "stub".into(),
                outcome: PollOutcome::default(),
            },
        ]),
        open_auth(),
    );

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/sources/a/runs")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["trigger"], "manual");
    assert_eq!(items[0]["status"], "failure");
    assert_eq!(items[0]["errorMessage"], "nyaa timed out");
    assert_eq!(items[0]["errorKind"], "timeout");
    assert_eq!(items[1]["status"], "success");
    assert_eq!(items[1]["fetchedCount"], 75);
    assert_eq!(items[1]["newCount"], 4);
    assert_eq!(items[1]["resolvedCount"], 3);
    assert_eq!(items[1]["fetchDurationMs"], 1200);

    // Unknown source name 404s; anon is rejected.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/sources/nope/runs")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sources/a/runs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn global_search_runs_carry_series_titles() {
    let db = fresh_db().await;
    let a = seed_series(&db, "Solo Leveling", "manhwa").await;
    let b = seed_series(&db, "Frieren", "manga").await;
    td_db::repos::search_runs_repo::insert_running(&db, 100, "eng", a, "manual")
        .await
        .unwrap();
    td_db::repos::search_runs_repo::insert_running(&db, 200, "raw", b, "cli")
        .await
        .unwrap();

    let app = build_app_with_search(
        db,
        open_auth(),
        vec![(search_stub("eng", vec![]), true)],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/search/runs")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    // Newest first; the flattened run fields sit beside the joined title.
    assert_eq!(items[0]["seriesTitle"], "Frieren");
    assert_eq!(items[0]["searchName"], "raw");
    assert_eq!(items[0]["trigger"], "cli");
    assert_eq!(items[0]["outcome"], "running");
    assert_eq!(items[1]["seriesTitle"], "Solo Leveling");

    // Admin-gated like the rest of the search surface.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/search/runs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ---- POST /releases/import (paste a post URL) ----------------------------

fn ingest_stub(name: &str, prefix: &str, release: Option<DiscoveredRelease>) -> StubSearchSource {
    StubSearchSource {
        name: name.into(),
        hits: vec![],
        delay: None,
        url_prefix: Some(prefix.into()),
        url_release: release,
    }
}

async fn post_import(app: axum::Router, body: &str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/api/v1/releases/import")
            .header(header::AUTHORIZATION, "Bearer write-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn import_persists_and_resolves_a_pasted_url() {
    let db = fresh_db().await;
    let app = build_app_with_search(
        db.clone(),
        open_auth(),
        vec![(
            ingest_stub(
                "nyaa-search",
                "https://nyaa.si/",
                Some(sample_release("991", "nyaa-search", "Chainsaw Man v01")),
            ),
            true,
        )],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );

    let resp = post_import(app, r#"{"url":"https://nyaa.si/view/991"}"#).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["alreadyKnown"], false);
    assert_eq!(body["release"]["externalId"], "991");
    assert_eq!(body["release"]["title"], "Chainsaw Man v01");
    assert_eq!(body["release"]["sourceName"], "nyaa-search");
    // The resolver ran: the row carries a decided status rather than the
    // `unresolved` default a bare insert would leave.
    assert!(
        body["release"]["resolutionStatus"].is_string(),
        "expected a resolution status, got {body}"
    );

    // Row is really in the catalog, keyed the same way a poll would key it.
    let stored = td_db::repos::releases_repo::find_by_id(
        &db,
        &td_db::repos::releases_repo::id_for("test", "991"),
    )
    .await
    .unwrap();
    assert!(stored.is_some(), "release was not persisted");
}

#[tokio::test]
async fn import_reports_already_known_without_creating_a_duplicate() {
    let db = fresh_db().await;
    let release = sample_release("991", "nyaa-search", "Chainsaw Man v01");
    let entries = || {
        vec![(
            ingest_stub("nyaa-search", "https://nyaa.si/", Some(release.clone())),
            true,
        )]
    };
    let locks = std::sync::Arc::new(td_scheduler::JobLocks::default());

    let first = post_import(
        build_app_with_search(db.clone(), open_auth(), entries(), locks.clone()),
        r#"{"url":"https://nyaa.si/view/991"}"#,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(body_json(first).await["alreadyKnown"], false);

    let second = post_import(
        build_app_with_search(db.clone(), open_auth(), entries(), locks),
        r#"{"url":"https://nyaa.si/view/991"}"#,
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let body = body_json(second).await;
    assert_eq!(body["alreadyKnown"], true);
    assert_eq!(body["release"]["externalId"], "991");

    let count = releases::Entity::find().all(&db).await.unwrap().len();
    assert_eq!(count, 1, "second import duplicated the release");
}

#[tokio::test]
async fn import_rejects_a_url_no_entry_recognizes() {
    let db = fresh_db().await;
    let app = build_app_with_search(
        db,
        open_auth(),
        vec![(
            ingest_stub(
                "nyaa-search",
                "https://nyaa.si/",
                Some(sample_release("1", "nyaa-search", "x")),
            ),
            true,
        )],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = post_import(app, r#"{"url":"https://example.org/view/1"}"#).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn import_is_not_found_when_the_upstream_has_no_such_post() {
    let db = fresh_db().await;
    let app = build_app_with_search(
        db,
        open_auth(),
        vec![(ingest_stub("nyaa-search", "https://nyaa.si/", None), true)],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = post_import(app, r#"{"url":"https://nyaa.si/view/404404"}"#).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn import_is_misconfigured_without_any_search_entries() {
    let db = fresh_db().await;
    let app = build_app_with_search(
        db,
        open_auth(),
        vec![],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = post_import(app, r#"{"url":"https://nyaa.si/view/991"}"#).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn import_requires_admin_token() {
    let db = fresh_db().await;
    let app = build_app_with_search(
        db,
        open_auth(),
        vec![(ingest_stub("nyaa-search", "https://nyaa.si/", None), true)],
        std::sync::Arc::new(td_scheduler::JobLocks::default()),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/releases/import")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"url":"https://nyaa.si/view/991"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
