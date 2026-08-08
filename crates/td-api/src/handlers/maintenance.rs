//! Admin-only housekeeping that deletes data.
//!
//! Everything here is manual and confirmed: no cron reaches these, by design.

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use td_db::repos::series_repo;
use utoipa::{IntoParams, ToSchema};

use crate::errors::{ApiError, ApiResult};
use crate::state::AppState;

/// How many rows the dry run shows the operator. Enough to recognise the
/// population without turning the confirmation into a wall of text.
const ORPHAN_SAMPLE_LIMIT: u64 = 20;

#[derive(Debug, Deserialize, IntoParams)]
#[serde(default, rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct OrphanSeriesQuery {
    /// Keep wishlisted series even when they are otherwise unreferenced.
    /// Defaults to `true`: a wishlisted orphan is usually a series the
    /// operator added by hand and is waiting on, so the safe reading of an
    /// absent parameter is "spare them".
    pub exclude_wishlisted: bool,
}

impl Default for OrphanSeriesQuery {
    fn default() -> Self {
        Self {
            exclude_wishlisted: true,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct PurgeOrphanSeriesRequest {
    /// See [`OrphanSeriesQuery::exclude_wishlisted`]. Same default, so a body
    /// that omits it cannot accidentally widen the purge.
    pub exclude_wishlisted: bool,
}

impl Default for PurgeOrphanSeriesRequest {
    fn default() -> Self {
        Self {
            exclude_wishlisted: true,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrphanSeriesRow {
    pub id: i32,
    pub canonical_title: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub first_seen_at: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrphanSeriesResponse {
    /// Total rows the purge would delete under these settings.
    pub count: u64,
    /// A bounded preview of them, oldest id first.
    pub sample: Vec<OrphanSeriesRow>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PurgeOrphanSeriesResponse {
    pub deleted: u64,
}

/// Dry run: what the orphan purge *would* delete.
///
/// "Orphan" is narrower than "has no releases". The resolver persists a
/// `series` row for every review candidate it records, so most release-less
/// series are the options the review queue is currently offering; deleting
/// those would empty the "pick the right match" panel. Series with a Codex
/// link, owned series, and (by default) wishlisted ones are spared too. See
/// `series_repo::orphan_series_condition`.
#[utoipa::path(
    get,
    path = "/api/v1/maintenance/orphan-series",
    tag = "maintenance",
    operation_id = "orphan_series_preview",
    params(OrphanSeriesQuery),
    responses((status = 200, body = OrphanSeriesResponse)),
    security(("admin" = []))
)]
pub async fn orphan_series_preview(
    State(state): State<AppState>,
    Query(q): Query<OrphanSeriesQuery>,
) -> ApiResult<Json<OrphanSeriesResponse>> {
    let count = series_repo::count_orphan_series(&state.db, q.exclude_wishlisted)
        .await
        .map_err(ApiError::Internal)?;
    let sample =
        series_repo::sample_orphan_series(&state.db, q.exclude_wishlisted, ORPHAN_SAMPLE_LIMIT)
            .await
            .map_err(ApiError::Internal)?;
    Ok(Json(OrphanSeriesResponse {
        count,
        sample: sample
            .into_iter()
            .map(|m| OrphanSeriesRow {
                id: m.id,
                canonical_title: m.canonical_title,
                kind: m.kind,
                first_seen_at: m.first_seen_at,
            })
            .collect(),
    }))
}

/// Delete the orphan series. **Irreversible** — there is no undo and no
/// tombstone; restoring means restoring the database file.
///
/// Uses the exact predicate the dry run counted, so what the operator
/// confirmed is what goes.
#[utoipa::path(
    post,
    path = "/api/v1/maintenance/orphan-series/purge",
    tag = "maintenance",
    operation_id = "purge_orphan_series",
    request_body = PurgeOrphanSeriesRequest,
    responses((status = 200, body = PurgeOrphanSeriesResponse)),
    security(("admin" = []))
)]
pub async fn purge_orphan_series(
    State(state): State<AppState>,
    Json(req): Json<PurgeOrphanSeriesRequest>,
) -> ApiResult<Json<PurgeOrphanSeriesResponse>> {
    let deleted = series_repo::purge_orphan_series(&state.db, req.exclude_wishlisted)
        .await
        .map_err(ApiError::Internal)?;
    tracing::info!(
        deleted,
        exclude_wishlisted = req.exclude_wishlisted,
        "purged orphan series"
    );
    Ok(Json(PurgeOrphanSeriesResponse { deleted }))
}
