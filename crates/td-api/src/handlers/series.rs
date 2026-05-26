//! Series read endpoints + manual `refresh-metadata` write.

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::Utc;
use sea_orm::{
    ColumnTrait, EntityTrait, JoinType, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait,
};
use serde::{Deserialize, Serialize};
use td_db::entities::{genres, series, series_external_ids, series_genres, series_tags, tags};
use td_db::repos::{series_external_ids_repo, tagging_repo};
use td_metadata::SeriesMetadata;
use td_resolution::persist;
use utoipa::{IntoParams, ToSchema};

use crate::errors::{ApiError, ApiResult};
use crate::handlers::pagination::Pagination;
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeriesListItem {
    pub id: i32,
    pub canonical_title: String,
    pub cover_url: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub year: Option<i32>,
    pub last_release_at: i64,
    pub first_seen_at: i64,
    pub owned: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeriesListPage {
    pub items: Vec<SeriesListItem>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIdDto {
    pub provider: String,
    pub external_id: String,
    pub external_url: Option<String>,
    pub fetched_at: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeriesDetail {
    pub id: i32,
    pub canonical_title: String,
    pub alternate_titles: Vec<String>,
    pub cover_url: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub year: Option<i32>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub metadata_source: String,
    pub metadata_fetched_at: i64,
    pub first_seen_at: i64,
    pub last_release_at: i64,
    pub highest_volume: Option<f64>,
    pub highest_chapter: Option<f64>,
    pub owned: bool,
    pub external_ids: Vec<ExternalIdDto>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(default, rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct SeriesListQuery {
    pub page: u32,
    pub page_size: u32,
    /// Filter by stored `series.type` (e.g. `manga`).
    pub kind: Option<String>,
    /// Filter by stored `series.status` (e.g. `ongoing`).
    pub status: Option<String>,
    /// Filter by ownership flag (true = owned by Codex, false = discoverable).
    pub owned: Option<bool>,
    /// Filter by a single genre name. AND-combined with the other filters.
    pub genre: Option<String>,
    /// Filter by a single tag name. AND-combined with the other filters.
    pub tag: Option<String>,
    /// Sort field. Supports `last_release_at` (default) and `first_seen_at`.
    pub sort: Option<String>,
    /// `asc` or `desc` (default).
    pub order: Option<String>,
}

impl Default for SeriesListQuery {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: 50,
            kind: None,
            status: None,
            owned: None,
            genre: None,
            tag: None,
            sort: None,
            order: None,
        }
    }
}

impl SeriesListQuery {
    fn pagination(&self) -> Pagination {
        Pagination {
            page: self.page,
            page_size: self.page_size,
        }
    }
}

/// List series ordered by last release timestamp (most recent first by default).
#[utoipa::path(
    get,
    path = "/api/v1/series",
    tag = "series",
    operation_id = "list_series",
    params(SeriesListQuery),
    responses((status = 200, body = SeriesListPage))
)]
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<SeriesListQuery>,
) -> ApiResult<Json<SeriesListPage>> {
    let pagination = q.pagination();
    let mut select = series::Entity::find();
    if let Some(k) = q.kind.as_deref() {
        select = select.filter(series::Column::Kind.eq(k));
    }
    if let Some(s) = q.status.as_deref() {
        select = select.filter(series::Column::Status.eq(s));
    }
    if let Some(owned) = q.owned {
        let flag = if owned { 1 } else { 0 };
        select = select.filter(series::Column::Owned.eq(flag));
    }
    // Genre/tag filters: AND-combined via two semi-join clauses. Names are
    // matched case-insensitively because the underlying UNIQUE constraints
    // collate NOCASE.
    if let Some(genre_name) = q.genre.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        select = select
            .join(
                JoinType::InnerJoin,
                series_genres::Relation::Series.def().rev(),
            )
            .join(JoinType::InnerJoin, series_genres::Relation::Genre.def())
            .filter(genres::Column::Name.eq(genre_name));
    }
    if let Some(tag_name) = q.tag.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        select = select
            .join(
                JoinType::InnerJoin,
                series_tags::Relation::Series.def().rev(),
            )
            .join(JoinType::InnerJoin, series_tags::Relation::Tag.def())
            .filter(tags::Column::Name.eq(tag_name));
    }
    let sort_col = match q.sort.as_deref() {
        Some("first_seen_at") => series::Column::FirstSeenAt,
        _ => series::Column::LastReleaseAt,
    };
    let desc = !matches!(q.order.as_deref(), Some("asc"));
    select = if desc {
        select.order_by_desc(sort_col)
    } else {
        select.order_by_asc(sort_col)
    };

    let total = select.clone().count(&state.db).await.map_err(anyhow_err)?;
    let rows = select
        .offset(pagination.offset())
        .limit(pagination.limit())
        .all(&state.db)
        .await
        .map_err(anyhow_err)?;

    let items: Vec<SeriesListItem> = rows.into_iter().map(model_to_list_item).collect();
    Ok(Json(SeriesListPage {
        items,
        page: pagination.page(),
        page_size: pagination.page_size(),
        total,
    }))
}

/// Series detail, including the resolved external-ID mappings.
#[utoipa::path(
    get,
    path = "/api/v1/series/{id}",
    tag = "series",
    params(("id" = i32, Path, description = "Internal series id")),
    responses(
        (status = 200, body = SeriesDetail),
        (status = 404, description = "No series with that id")
    )
)]
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> ApiResult<Json<SeriesDetail>> {
    let row = series::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(anyhow_err)?
        .ok_or_else(|| ApiError::NotFound(format!("series {id}")))?;
    let mappings = series_external_ids_repo::list_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    let tags_for_series = tagging_repo::list_tags_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    let genres_for_series = tagging_repo::list_genres_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    Ok(Json(model_to_detail(
        row,
        mappings,
        genres_for_series,
        tags_for_series,
    )))
}

/// Re-fetch metadata for a series from the active provider and re-persist.
#[utoipa::path(
    post,
    path = "/api/v1/series/{id}/refresh-metadata",
    tag = "series",
    params(("id" = i32, Path, description = "Internal series id")),
    responses(
        (status = 200, body = SeriesDetail),
        (status = 404, description = "Series or provider entry not found"),
        (status = 409, description = "No mapping for the active provider on this series")
    ),
    security(("admin" = []))
)]
pub async fn refresh_metadata(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> ApiResult<Json<SeriesDetail>> {
    let _ = series::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(anyhow_err)?
        .ok_or_else(|| ApiError::NotFound(format!("series {id}")))?;

    let active_id = state.metadata.active_id().to_string();
    let active = state.metadata.active().clone();
    let mappings = series_external_ids_repo::list_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    let Some(active_mapping) = mappings.iter().find(|m| m.provider == active_id) else {
        return Err(ApiError::Conflict(format!(
            "series {id} has no mapping for active provider {active_id:?}; link it manually first"
        )));
    };

    let metadata: SeriesMetadata = active
        .get(&active_mapping.external_id)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("active.get failed: {e}")))?
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "active provider {:?} has no record for {}",
                active_id, active_mapping.external_id
            ))
        })?;

    let now = Utc::now();
    persist::upsert_series_from_metadata(&state.db, &active_id, &metadata, now.timestamp(), now)
        .await
        .map_err(ApiError::Internal)?;

    let row = series::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(anyhow_err)?
        .ok_or_else(|| ApiError::NotFound(format!("series {id}")))?;
    let mappings = series_external_ids_repo::list_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    let tags_for_series = tagging_repo::list_tags_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    let genres_for_series = tagging_repo::list_genres_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    Ok(Json(model_to_detail(
        row,
        mappings,
        genres_for_series,
        tags_for_series,
    )))
}

fn model_to_list_item(m: series::Model) -> SeriesListItem {
    SeriesListItem {
        id: m.id,
        canonical_title: m.canonical_title,
        cover_url: m.cover_url,
        kind: m.kind,
        status: m.status,
        year: m.year,
        last_release_at: m.last_release_at,
        first_seen_at: m.first_seen_at,
        owned: m.owned != 0,
    }
}

fn model_to_detail(
    m: series::Model,
    mappings: Vec<series_external_ids::Model>,
    join_genres: Vec<String>,
    join_tags: Vec<String>,
) -> SeriesDetail {
    let alternate_titles = m
        .alternate_titles_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default();
    // Prefer the normalized join table. Fall back to the JSON column for
    // legacy rows the backfill couldn't lift (malformed JSON, etc.).
    let genres = if !join_genres.is_empty() {
        join_genres
    } else {
        m.genres_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .unwrap_or_default()
    };
    SeriesDetail {
        id: m.id,
        canonical_title: m.canonical_title,
        alternate_titles,
        cover_url: m.cover_url,
        kind: m.kind,
        status: m.status,
        year: m.year,
        genres,
        tags: join_tags,
        metadata_source: m.metadata_source,
        metadata_fetched_at: m.metadata_fetched_at,
        first_seen_at: m.first_seen_at,
        last_release_at: m.last_release_at,
        highest_volume: m.highest_volume,
        highest_chapter: m.highest_chapter,
        owned: m.owned != 0,
        external_ids: mappings
            .into_iter()
            .map(|x| ExternalIdDto {
                provider: x.provider,
                external_id: x.external_id,
                external_url: x.external_url,
                fetched_at: x.fetched_at,
            })
            .collect(),
    }
}

fn anyhow_err<E: Into<anyhow::Error>>(e: E) -> ApiError {
    ApiError::Internal(e.into())
}
