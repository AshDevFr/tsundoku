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
use td_db::repos::{releases_repo, series_external_ids_repo, tagging_repo};
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
        genres_json: Set(None),
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
    series_external_ids_repo::upsert(&db, sid, "mb", "42", Some("https://mb/42"), 100)
        .await
        .unwrap();
    series_external_ids_repo::upsert(&db, sid, "anilist", "9", None, 100)
        .await
        .unwrap();

    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
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
async fn refresh_all_returns_per_provider_results() {
    let db = fresh_db().await;
    let app = build_app(
        db,
        metadata_registry_with(StubProvider {
            id: "mb",
            returns: None,
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
        }),
        source_registry_with(vec![]),
        open_auth(),
    );

    // genre filter alone returns both Action rows.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/series?genre=Action")
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
                .uri("/api/v1/series?genre=Action&tag=isekai")
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
                .uri("/api/v1/series?genre=nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["total"], 0);
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
