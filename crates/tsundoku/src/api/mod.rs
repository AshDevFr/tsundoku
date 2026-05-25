pub mod docs;
pub mod health;
pub mod static_files;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use sea_orm::DatabaseConnection;
use td_config::AppConfig;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
}

/// Build the application router.
///
/// `/api/v1/*` carries the JSON API; everything else falls through to the
/// embedded SPA so client-side routing works on hard refresh. The SPA fallback
/// must stay last.
pub fn router(state: Arc<AppState>, cfg: &AppConfig) -> Router {
    let api = Router::new()
        .route("/health", get(health::health))
        .with_state(state);

    let mut app = Router::new()
        .nest("/api/v1", api)
        .fallback(get(static_files::serve_static))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    if cfg.api.docs {
        app = app.merge(Scalar::with_url("/docs", docs::ApiDoc::openapi()));
    }
    app
}
