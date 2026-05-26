//! Top-level axum router.
//!
//! Reads are mounted under `/api/v1` and (optionally) layered with the
//! [`auth::require_read`] middleware. Writes share the same `/api/v1`
//! prefix but layer the [`auth::require_admin`] check unconditionally.

use axum::Router;
use axum::middleware;
use axum::routing::{get, post};
use td_config::AppConfig;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};

use crate::auth;
use crate::docs::ApiDoc;
use crate::embed::serve_static;
use crate::handlers::{health, metrics, providers, releases, series, sources, stats, tagging};
use crate::state::AppState;

pub fn router(state: AppState, cfg: &AppConfig) -> Router {
    let auth = state.auth.clone();

    let writes = Router::new()
        .route(
            "/series/{id}/refresh-metadata",
            post(series::refresh_metadata),
        )
        .route("/releases/{id}/link", post(releases::link))
        .route("/releases/{id}/reject", post(releases::reject))
        .route("/releases/{id}/retry", post(releases::retry))
        .route("/releases/retry-all", post(releases::retry_all))
        .route("/sources/{name}/poll", post(sources::poll))
        .route("/sources/poll-all", post(sources::poll_all))
        .route(
            "/providers/{id}/refresh-cache",
            post(providers::refresh_cache),
        )
        .route("/providers/refresh-all", post(providers::refresh_all))
        .route_layer(middleware::from_fn_with_state(
            auth.clone(),
            auth::require_admin,
        ));

    let reads = Router::new()
        .route("/health", get(health::health))
        .route("/stats", get(stats::stats))
        .route("/series", get(series::list))
        .route("/series/{id}", get(series::get))
        .route("/releases", get(releases::list))
        .route("/releases/unresolved", get(releases::list_unresolved))
        .route("/sources", get(sources::list))
        .route("/providers", get(providers::list))
        .route("/providers/{id}/search", get(providers::search))
        .route("/genres", get(tagging::list_genres))
        .route("/tags", get(tagging::list_tags))
        .route("/metrics/sources", get(metrics::sources_summary))
        .route("/metrics/sources/{name}", get(metrics::sources_detail))
        .route("/metrics/providers", get(metrics::providers_summary))
        .route("/metrics/providers/{id}", get(metrics::providers_detail))
        .route("/metrics/review-queue", get(metrics::review_queue))
        .route("/metrics/id-maps", get(metrics::id_maps))
        .route_layer(middleware::from_fn_with_state(
            auth.clone(),
            auth::require_read,
        ));

    let api = reads.merge(writes).with_state(state);

    let mut app = Router::new()
        .nest("/api/v1", api)
        .fallback(get(serve_static))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    if cfg.api.docs {
        app = app.merge(Scalar::with_url("/docs", ApiDoc::openapi()));
    }
    app
}
