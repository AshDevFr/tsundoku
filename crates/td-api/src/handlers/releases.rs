//! Release endpoints: list / unresolved feed / link / reject / retry.

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::Utc;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use td_db::entities::releases;
use td_db::repos::{releases_repo, review_repo};
use td_metadata::SeriesMetadata;
use td_resolution::{Resolver, persist};
use utoipa::{IntoParams, ToSchema};

use crate::errors::{ApiError, ApiResult};
use crate::handlers::pagination::Pagination;
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseDto {
    pub id: String,
    pub source_kind: String,
    pub source_name: String,
    pub external_id: String,
    pub title: String,
    pub link: String,
    pub magnet: Option<String>,
    pub torrent_url: Option<String>,
    pub ddl_url: Option<String>,
    pub info_hash: Option<String>,
    pub size_bytes: Option<i64>,
    pub files: Vec<String>,
    pub formats: Vec<String>,
    pub posted_at: i64,
    pub observed_at: i64,
    pub series_id: Option<i32>,
    pub resolution_path: Option<String>,
    pub resolution_confidence: Option<f64>,
    pub resolution_status: String,
    pub resolution_attempts: i32,
    pub last_resolve_attempt_at: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReleasePage {
    pub items: Vec<ReleaseDto>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCandidateDto {
    pub series_id: i32,
    pub series_title: String,
    pub series_cover_url: Option<String>,
    pub score: f64,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedRelease {
    #[serde(flatten)]
    pub release: ReleaseDto,
    pub candidates: Vec<ReviewCandidateDto>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedPage {
    pub items: Vec<UnresolvedRelease>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(default, rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct ReleaseListQuery {
    pub page: u32,
    pub page_size: u32,
    /// Filter by resolution status (`resolved`, `unresolved`, `ambiguous`, `review_pending`).
    pub status: Option<String>,
    pub source_kind: Option<String>,
    pub source_name: Option<String>,
    pub series_id: Option<i32>,
}

impl Default for ReleaseListQuery {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: 50,
            status: None,
            source_kind: None,
            source_name: None,
            series_id: None,
        }
    }
}

impl ReleaseListQuery {
    fn pagination(&self) -> Pagination {
        Pagination {
            page: self.page,
            page_size: self.page_size,
        }
    }
}

/// List releases ordered by `observed_at` descending. Filters compose.
#[utoipa::path(
    get,
    path = "/api/v1/releases",
    tag = "releases",
    params(ReleaseListQuery),
    responses((status = 200, body = ReleasePage))
)]
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<ReleaseListQuery>,
) -> ApiResult<Json<ReleasePage>> {
    let pagination = q.pagination();
    let mut select = releases::Entity::find();
    if let Some(s) = q.status.as_deref() {
        select = select.filter(releases::Column::ResolutionStatus.eq(s));
    }
    if let Some(s) = q.source_kind.as_deref() {
        select = select.filter(releases::Column::SourceKind.eq(s));
    }
    if let Some(s) = q.source_name.as_deref() {
        select = select.filter(releases::Column::SourceName.eq(s));
    }
    if let Some(id) = q.series_id {
        select = select.filter(releases::Column::SeriesId.eq(id));
    }
    let total = select.clone().count(&state.db).await.map_err(anyhow_err)?;
    let rows = select
        .order_by_desc(releases::Column::ObservedAt)
        .offset(pagination.offset())
        .limit(pagination.limit())
        .all(&state.db)
        .await
        .map_err(anyhow_err)?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let formats = releases_repo::list_formats(&state.db, &row.id)
            .await
            .map_err(anyhow_err)?;
        items.push(model_to_release(row, formats));
    }

    Ok(Json(ReleasePage {
        items,
        page: pagination.page(),
        page_size: pagination.page_size(),
        total,
    }))
}

/// Review queue: releases awaiting human attention.
///
/// Returns releases whose status is `unresolved`, `ambiguous`, or
/// `review_pending`, each with the recorded review candidates so the UI
/// can render a "pick the right match" panel without a second fetch.
#[utoipa::path(
    get,
    path = "/api/v1/releases/unresolved",
    tag = "releases",
    params(Pagination),
    responses((status = 200, body = UnresolvedPage))
)]
pub async fn list_unresolved(
    State(state): State<AppState>,
    Query(p): Query<Pagination>,
) -> ApiResult<Json<UnresolvedPage>> {
    let select = releases::Entity::find().filter(releases::Column::ResolutionStatus.is_in([
        "unresolved",
        "ambiguous",
        "review_pending",
    ]));
    let total = select.clone().count(&state.db).await.map_err(anyhow_err)?;
    let rows = select
        .order_by_desc(releases::Column::ObservedAt)
        .offset(p.offset())
        .limit(p.limit())
        .all(&state.db)
        .await
        .map_err(anyhow_err)?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let formats = releases_repo::list_formats(&state.db, &row.id)
            .await
            .map_err(anyhow_err)?;
        let candidate_rows = review_repo::list_for_release(&state.db, &row.id)
            .await
            .map_err(anyhow_err)?;
        let mut candidates = Vec::with_capacity(candidate_rows.len());
        for c in candidate_rows {
            let series = td_db::repos::series_repo::find_by_id(&state.db, c.series_id)
                .await
                .map_err(anyhow_err)?;
            candidates.push(ReviewCandidateDto {
                series_id: c.series_id,
                series_title: series
                    .as_ref()
                    .map(|s| s.canonical_title.clone())
                    .unwrap_or_default(),
                series_cover_url: series.and_then(|s| s.cover_url),
                score: c.score,
                reason: c.reason,
            });
        }
        items.push(UnresolvedRelease {
            release: model_to_release(row, formats),
            candidates,
        });
    }

    Ok(Json(UnresolvedPage {
        items,
        page: p.page(),
        page_size: p.page_size(),
        total,
    }))
}

/// Body for the manual-link endpoint. Exactly one of:
/// - `seriesId`: link to an existing series row by internal id.
/// - `provider` + `externalId`: link via the named provider's external id;
///   the provider's `get` is called when no mapping exists yet.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LinkRequest {
    pub series_id: Option<i32>,
    pub provider: Option<String>,
    pub external_id: Option<String>,
}

/// Manually link a release to a series. Body shape:
///
/// - `{ "seriesId": 42 }` — link to an existing series row by internal id.
/// - `{ "provider": "mangabaka", "externalId": "1677" }` — link by a
///   provider's external id. If no `series_external_ids` row matches yet,
///   the active provider's `get` is called to fetch metadata and create
///   the series row before linking.
#[utoipa::path(
    post,
    path = "/api/v1/releases/{id}/link",
    tag = "releases",
    params(("id" = String, Path, description = "Release id")),
    request_body = LinkRequest,
    responses(
        (status = 200, body = ReleaseDto),
        (status = 400, description = "Provider not registered or external_id unknown to provider"),
        (status = 404, description = "Release or series not found")
    ),
    security(("admin" = []))
)]
pub async fn link(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<LinkRequest>,
) -> ApiResult<Json<ReleaseDto>> {
    let release = releases_repo::find_by_id(&state.db, &id)
        .await
        .map_err(anyhow_err)?
        .ok_or_else(|| ApiError::NotFound(format!("release {id:?}")))?;

    let now = Utc::now();
    let attempted_at = now.timestamp();
    let series_id = resolve_link_target(&state, &release, req, now).await?;

    // Clear any stale review candidates for this release: the operator
    // just made a decision.
    review_repo::replace_for_release(&state.db, &release.id, Vec::new())
        .await
        .map_err(anyhow_err)?;
    persist::link_release(
        &state.db,
        &release.id,
        Some(series_id),
        Some("manual"),
        Some(1.0),
        "resolved",
        attempted_at,
    )
    .await
    .map_err(ApiError::Internal)?;

    let row = releases_repo::find_by_id(&state.db, &release.id)
        .await
        .map_err(anyhow_err)?
        .ok_or_else(|| ApiError::NotFound(format!("release {id:?}")))?;
    let formats = releases_repo::list_formats(&state.db, &row.id)
        .await
        .map_err(anyhow_err)?;
    Ok(Json(model_to_release(row, formats)))
}

/// Mark a release as "not a series we care about". Drops candidates and
/// pins the resolution status to `rejected` so the resolver leaves it
/// alone on subsequent runs.
#[utoipa::path(
    post,
    path = "/api/v1/releases/{id}/reject",
    tag = "releases",
    params(("id" = String, Path, description = "Release id")),
    responses(
        (status = 200, body = ReleaseDto),
        (status = 404, description = "Release not found")
    ),
    security(("admin" = []))
)]
pub async fn reject(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<ReleaseDto>> {
    let _ = releases_repo::find_by_id(&state.db, &id)
        .await
        .map_err(anyhow_err)?
        .ok_or_else(|| ApiError::NotFound(format!("release {id:?}")))?;
    review_repo::replace_for_release(&state.db, &id, Vec::new())
        .await
        .map_err(anyhow_err)?;
    let now = Utc::now().timestamp();
    persist::link_release(
        &state.db,
        &id,
        None,
        Some("rejected"),
        None,
        "rejected",
        now,
    )
    .await
    .map_err(ApiError::Internal)?;

    let row = releases_repo::find_by_id(&state.db, &id)
        .await
        .map_err(anyhow_err)?
        .ok_or_else(|| ApiError::NotFound(format!("release {id:?}")))?;
    let formats = releases_repo::list_formats(&state.db, &row.id)
        .await
        .map_err(anyhow_err)?;
    Ok(Json(model_to_release(row, formats)))
}

/// Re-run the resolver against a single release. Useful after a provider
/// refresh, a config change, or a manual edit.
#[utoipa::path(
    post,
    path = "/api/v1/releases/{id}/retry",
    tag = "releases",
    params(("id" = String, Path, description = "Release id")),
    responses(
        (status = 200, body = ReleaseDto),
        (status = 404, description = "Release not found")
    ),
    security(("admin" = []))
)]
pub async fn retry(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<ReleaseDto>> {
    let _ = releases_repo::find_by_id(&state.db, &id)
        .await
        .map_err(anyhow_err)?
        .ok_or_else(|| ApiError::NotFound(format!("release {id:?}")))?;
    let resolver = Resolver::new(
        state.db.clone(),
        state.metadata.clone(),
        state.ingestion.clone(),
    );
    resolver
        .resolve_one(&id)
        .await
        .map_err(ApiError::Internal)?;

    let row = releases_repo::find_by_id(&state.db, &id)
        .await
        .map_err(anyhow_err)?
        .ok_or_else(|| ApiError::NotFound(format!("release {id:?}")))?;
    let formats = releases_repo::list_formats(&state.db, &row.id)
        .await
        .map_err(anyhow_err)?;
    Ok(Json(model_to_release(row, formats)))
}

async fn resolve_link_target(
    state: &AppState,
    release: &releases::Model,
    req: LinkRequest,
    now: chrono::DateTime<Utc>,
) -> ApiResult<i32> {
    match (req.series_id, req.provider, req.external_id) {
        (Some(sid), None, None) => {
            td_db::repos::series_repo::find_by_id(&state.db, sid)
                .await
                .map_err(anyhow_err)?
                .ok_or_else(|| ApiError::NotFound(format!("series {sid}")))?;
            Ok(sid)
        }
        (None, Some(provider), Some(external_id)) => {
            if let Some(sid) = td_db::repos::series_external_ids_repo::find_series_id(
                &state.db,
                &provider,
                &external_id,
            )
            .await
            .map_err(anyhow_err)?
            {
                return Ok(sid);
            }
            let target = state.metadata.get(&provider).ok_or_else(|| {
                ApiError::BadRequest(format!("provider {provider:?} not registered"))
            })?;
            let metadata: SeriesMetadata = target
                .get(&external_id)
                .await
                .map_err(|e| ApiError::Internal(anyhow::anyhow!("provider.get failed: {e}")))?
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "provider {provider:?} has no record for {external_id:?}"
                    ))
                })?;
            Ok(persist::upsert_series_from_metadata(
                &state.db,
                &provider,
                &metadata,
                release.posted_at,
                now,
            )
            .await
            .map_err(ApiError::Internal)?
            .series_id)
        }
        _ => Err(ApiError::BadRequest(
            "body must set either `seriesId` or both `provider` and `externalId`".into(),
        )),
    }
}

fn model_to_release(m: releases::Model, formats: Vec<String>) -> ReleaseDto {
    let files = m
        .files_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default();
    ReleaseDto {
        id: m.id,
        source_kind: m.source_kind,
        source_name: m.source_name,
        external_id: m.external_id,
        title: m.title,
        link: m.link,
        magnet: m.magnet,
        torrent_url: m.torrent_url,
        ddl_url: m.ddl_url,
        info_hash: m.info_hash,
        size_bytes: m.size_bytes,
        files,
        formats,
        posted_at: m.posted_at,
        observed_at: m.observed_at,
        series_id: m.series_id,
        resolution_path: m.resolution_path,
        resolution_confidence: m.resolution_confidence,
        resolution_status: m.resolution_status,
        resolution_attempts: m.resolution_attempts,
        last_resolve_attempt_at: m.last_resolve_attempt_at,
    }
}

fn anyhow_err<E: Into<anyhow::Error>>(e: E) -> ApiError {
    ApiError::Internal(e.into())
}
