//! `/api/v1/genres` and `/api/v1/tags` listing endpoints.
//!
//! Returns the canonical list of genre / tag names + per-name series count so
//! the feed UI can populate a usage-sorted dropdown.

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use td_db::repos::tagging_repo;
use utoipa::ToSchema;

use crate::errors::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagUsageDto {
    pub name: String,
    pub series_count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagList {
    pub items: Vec<TagUsageDto>,
}

/// Every genre that's been observed at least once, sorted by descending
/// series count, then name. Powers the feed-filter autocomplete.
#[utoipa::path(
    get,
    path = "/api/v1/genres",
    tag = "tagging",
    operation_id = "list_genres",
    responses((status = 200, body = TagList))
)]
pub async fn list_genres(State(state): State<AppState>) -> ApiResult<Json<TagList>> {
    let rows = tagging_repo::list_genres_with_counts(&state.db)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(TagList {
        items: rows
            .into_iter()
            .map(|r| TagUsageDto {
                name: r.name,
                series_count: r.series_count,
            })
            .collect(),
    }))
}

/// Tag analog of [`list_genres`]. Same shape, different table.
#[utoipa::path(
    get,
    path = "/api/v1/tags",
    tag = "tagging",
    operation_id = "list_tags",
    responses((status = 200, body = TagList))
)]
pub async fn list_tags(State(state): State<AppState>) -> ApiResult<Json<TagList>> {
    let rows = tagging_repo::list_tags_with_counts(&state.db)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(TagList {
        items: rows
            .into_iter()
            .map(|r| TagUsageDto {
                name: r.name,
                series_count: r.series_count,
            })
            .collect(),
    }))
}
