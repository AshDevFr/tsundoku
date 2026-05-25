use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;
use utoipa::ToSchema;

use super::AppState;

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub status: &'static str,
}

/// Liveness check. Pings the database and reports `ok`.
#[utoipa::path(
    get,
    path = "/api/v1/health",
    tag = "tsundoku",
    responses((status = 200, body = Health), (status = 503, description = "Database unreachable"))
)]
pub async fn health(State(state): State<Arc<AppState>>) -> Result<Json<Health>, StatusCode> {
    state
        .db
        .ping()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(Health { status: "ok" }))
}
