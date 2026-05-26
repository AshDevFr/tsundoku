//! Metadata-provider listing + manual cache-refresh trigger.

use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use td_db::repos::provider_cache_state_repo;
use td_scheduler::jobs::refresh_provider_cache;
use utoipa::ToSchema;

use crate::errors::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCacheState {
    pub fetched_at: i64,
    pub cache_version: Option<String>,
    pub record_count: Option<i64>,
    pub bytes_downloaded: Option<i64>,
    pub source_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDto {
    pub id: String,
    pub display_name: String,
    pub active: bool,
    pub last_refresh: Option<ProviderCacheState>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderList {
    pub items: Vec<ProviderDto>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResponse {
    pub provider: String,
    pub triggered: bool,
    /// `false` when a refresh is already in flight; the request is a no-op.
    pub skipped: bool,
}

/// List every registered metadata provider with its latest cache-refresh
/// markers and the active-provider flag.
#[utoipa::path(
    get,
    path = "/api/v1/providers",
    tag = "providers",
    responses((status = 200, body = ProviderList))
)]
pub async fn list(State(state): State<AppState>) -> ApiResult<Json<ProviderList>> {
    let active = state.metadata.active_id().to_string();
    let mut items = Vec::with_capacity(state.metadata.len());
    for (id, provider) in state.metadata.iter() {
        let latest = provider_cache_state_repo::latest(&state.db, id)
            .await
            .map_err(ApiError::Internal)?;
        items.push(ProviderDto {
            id: id.to_string(),
            display_name: provider.display_name().to_string(),
            active: id == active,
            last_refresh: latest.map(|r| ProviderCacheState {
                fetched_at: r.fetched_at,
                cache_version: r.cache_version,
                record_count: r.record_count,
                bytes_downloaded: r.bytes_downloaded,
                source_url: r.source_url,
            }),
        });
    }
    items.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Json(ProviderList { items }))
}

/// Trigger an offline-cache refresh for the named provider. Same locking
/// semantics as the scheduled refresh job; an in-flight refresh causes
/// this request to report `skipped = true` without spawning a duplicate.
#[utoipa::path(
    post,
    path = "/api/v1/providers/{id}/refresh-cache",
    tag = "providers",
    params(("id" = String, Path, description = "Provider id")),
    responses(
        (status = 202, body = RefreshResponse),
        (status = 404, description = "No provider with that id registered")
    ),
    security(("admin" = []))
)]
pub async fn refresh_cache(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<RefreshResponse>> {
    let provider = state
        .metadata
        .get(&id)
        .cloned()
        .ok_or_else(|| ApiError::NotFound(format!("provider {id:?}")))?;

    let lock = state.locks.provider_lock(&id);
    let skipped = lock.try_lock().is_err();
    if skipped {
        return Ok(Json(RefreshResponse {
            provider: id,
            triggered: false,
            skipped: true,
        }));
    }
    // Drop the test-lock; the spawned tick re-acquires it. Same race
    // tolerance as the sources handler.

    let db = state.db.clone();
    let locks = state.locks.clone();
    tokio::spawn(async move {
        refresh_provider_cache::run_tick(provider, db, locks).await;
    });

    Ok(Json(RefreshResponse {
        provider: id,
        triggered: true,
        skipped: false,
    }))
}
