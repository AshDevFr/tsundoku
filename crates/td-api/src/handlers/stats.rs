//! Service-wide counters and last-activity markers.

use axum::Json;
use axum::extract::State;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde::Serialize;
use td_db::entities::{releases, series};
use utoipa::ToSchema;

use crate::errors::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseCounts {
    pub resolved: u64,
    pub unresolved: u64,
    pub ambiguous: u64,
    pub review_pending: u64,
    pub rejected: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatsResponse {
    pub series: u64,
    pub releases: ReleaseCounts,
    pub total_releases: u64,
    pub active_provider: String,
}

/// Aggregate counts surfaced by the frontend home page and review badge.
#[utoipa::path(
    get,
    path = "/api/v1/stats",
    tag = "system",
    responses((status = 200, body = StatsResponse))
)]
pub async fn stats(State(state): State<AppState>) -> ApiResult<Json<StatsResponse>> {
    let series_total = series::Entity::find()
        .count(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let counts = ReleaseCounts {
        resolved: count_status(&state, "resolved").await?,
        unresolved: count_status(&state, "unresolved").await?,
        ambiguous: count_status(&state, "ambiguous").await?,
        review_pending: count_status(&state, "review_pending").await?,
        rejected: count_status(&state, "rejected").await?,
    };
    let total_releases = releases::Entity::find()
        .count(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    Ok(Json(StatsResponse {
        series: series_total,
        releases: counts,
        total_releases,
        active_provider: state.metadata.active_id().to_string(),
    }))
}

async fn count_status(state: &AppState, status: &str) -> ApiResult<u64> {
    releases::Entity::find()
        .filter(releases::Column::ResolutionStatus.eq(status))
        .count(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))
}
