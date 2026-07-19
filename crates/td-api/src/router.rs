//! Top-level axum router.
//!
//! Reads are mounted under `/api/v1` and (optionally) layered with the
//! [`auth::require_read`] middleware. Writes share the same `/api/v1`
//! prefix but layer the [`auth::require_admin`] check unconditionally.

use axum::Router;
use axum::middleware;
use axum::routing::{delete, get, patch, post, put};
use td_config::AppConfig;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};

use crate::auth;
use crate::docs::ApiDoc;
use crate::embed::serve_static;
use crate::handlers::{
    codex, covers, download, events, health, info, metrics, providers, releases, search, series,
    series_export, sources, stats, tagging,
};
use crate::state::AppState;

pub fn router(state: AppState, cfg: &AppConfig) -> Router {
    let auth = state.auth.clone();

    let writes = Router::new()
        .route("/series", post(series::create))
        .route("/series/from-provider", post(series::create_from_provider))
        .route("/series/{id}", patch(series::update))
        .route("/series/refresh-all", post(series::refresh_all))
        // Catalog export: admin-only (exposes Codex ownership), so it lives in
        // the writes group despite being a GET — same rationale as
        // `GET /codex/status` and `GET /download/status`.
        .route("/series/export", get(series_export::export))
        .route("/series/recompute-spans", post(series::recompute_spans))
        .route(
            "/series/invalidate-metadata-hashes",
            post(series::invalidate_metadata_hashes),
        )
        .route(
            "/series/{id}/refresh-metadata",
            post(series::refresh_metadata),
        )
        .route(
            "/series/{id}/ignore-completion",
            put(series::set_ignore_completion),
        )
        .route("/series/{id}/wishlist", put(series::set_wishlisted))
        // Series bulk actions (static `bulk` segment wins over `{id}` in
        // axum's matcher, same coexistence as `/series/lookup`).
        .route("/series/bulk/wishlist", put(series::bulk_wishlist))
        .route(
            "/series/bulk/refresh-metadata",
            post(series::bulk_refresh_metadata),
        )
        .route("/releases/{id}/link", post(releases::link))
        .route("/releases/{id}/reject", post(releases::reject))
        .route("/releases/{id}/keep", post(releases::keep))
        .route("/releases/{id}/retry", post(releases::retry))
        .route("/releases/retry-all", post(releases::retry_all))
        .route("/releases/bulk/reject", post(releases::bulk_reject))
        .route("/releases/bulk/retry", post(releases::bulk_retry))
        .route("/releases/bulk/link", post(releases::bulk_link))
        .route("/sources/{name}/poll", post(sources::poll))
        .route("/sources/{name}/backfill", post(sources::backfill))
        .route("/sources/{name}/re-enrich", post(sources::reenrich))
        .route("/sources/poll-all", post(sources::poll_all))
        // Source-filter dropdown vocab. Admin-only (in this group) because the
        // series-list `source` filter it feeds is itself admin-only.
        .route(
            "/sources/with-series-count",
            get(sources::list_with_series_counts),
        )
        .route(
            "/providers/{id}/refresh-cache",
            post(providers::refresh_cache),
        )
        .route("/providers/refresh-all", post(providers::refresh_all))
        .route("/covers/invalidate-cache", post(covers::invalidate_cache))
        // Codex presence: admin-only. `GET /codex/status` lives here (not in
        // the reads group) because it exposes what is in the operator's Codex
        // library, which must never reach the public read tier.
        .route("/codex/refresh", post(codex::refresh))
        .route("/codex/status", get(codex::status))
        .route("/codex/test", post(codex::test))
        .route("/series/{id}/codex-link", post(codex::link))
        .route("/series/{id}/codex-link", delete(codex::unlink))
        // Send to torrent client: admin-only. `GET /download/status` lives here
        // (not in the reads group) so the enablement probe stays behind admin
        // auth, like `GET /codex/status`.
        .route("/releases/{id}/send-to-client", post(download::send))
        .route("/download/status", get(download::status))
        .route("/download/test", post(download::test))
        // Per-series release search: admin-only, entries list included —
        // it exposes config naming, which stays off the public read tier.
        .route("/search/entries", get(search::entries))
        .route("/search/runs", get(search::global_runs))
        .route("/series/{id}/search-releases", post(search::trigger))
        .route("/series/bulk/search-releases", post(search::bulk_trigger))
        .route("/series/{id}/search-runs", get(search::runs))
        // Per-run history behind the aggregated metrics: admin-only like
        // every other run/audit surface (error messages stay private).
        .route("/sources/{name}/runs", get(sources::runs))
        .route_layer(middleware::from_fn_with_state(
            auth.clone(),
            auth::require_admin,
        ));

    let reads = Router::new()
        .route("/health", get(health::health))
        .route("/info", get(info::info))
        .route("/stats", get(stats::stats))
        .route("/series", get(series::list))
        .route("/series/feed", get(series::feed).post(series::feed_query))
        // External-ID deep-link resolver; static segment, so it wins over
        // `/series/{id}` in axum's matcher.
        .route("/series/lookup", get(series::lookup))
        .route("/series/{id}", get(series::get))
        .route("/releases", get(releases::list))
        .route("/releases/unresolved", get(releases::list_unresolved))
        .route("/releases/unresolved/groups", get(releases::list_groups))
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
        .route("/events/jobs", get(events::jobs))
        .route("/covers/by-url", get(covers::get_by_url))
        .route("/covers/{series_id}", get(covers::get_by_series_id))
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
