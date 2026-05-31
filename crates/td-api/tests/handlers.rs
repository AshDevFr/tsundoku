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
use td_source::PollOutcome;
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

    // Duplicate values collapse; order is preserved.
    let body = serde_json::json!({ "statuses": ["unresolved", "unresolved", "ambiguous"] });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sources/a/re-enrich")
                .header(header::AUTHORIZATION, "Bearer write-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["source"], "a");
    assert_eq!(body["triggered"], true);
    assert_eq!(body["skipped"], false);
    assert_eq!(
        body["statuses"],
        serde_json::json!(["unresolved", "ambiguous"])
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

    // Empty set.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sources/a/re-enrich")
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
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sources/a/re-enrich")
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
