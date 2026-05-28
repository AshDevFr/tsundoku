//! App metadata (name + version).

use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
}

/// Returns the running binary's name and semver. Cheap, public, cache-forever.
#[utoipa::path(
    get,
    path = "/api/v1/info",
    tag = "system",
    responses((status = 200, body = AppInfo))
)]
pub async fn info() -> Json<AppInfo> {
    Json(AppInfo {
        name: "tsundoku",
        version: env!("CARGO_PKG_VERSION"),
    })
}
