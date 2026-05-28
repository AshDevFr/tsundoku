//! Series read endpoints + manual `refresh-metadata` write.

use std::collections::HashSet;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::Utc;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
    Set,
};
use serde::{Deserialize, Serialize};
use td_db::entities::{
    genres, releases, series, series_external_ids, series_genres, series_tags, tags,
};
use td_db::repos::run_metrics_repo;
use td_db::repos::{releases_repo, series_external_ids_repo, series_repo, tagging_repo};
use td_metadata::SeriesMetadata;
use td_metadata::scoring::best_dice;
use td_resolution::persist;
use td_scheduler::dispatch;
use td_scheduler::jobs::refresh_series_metadata;
use utoipa::{IntoParams, ToSchema};

use crate::errors::{ApiError, ApiResult};
use crate::handlers::pagination::Pagination;
use crate::state::{AppState, JobKind, JobResult};

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeriesListItem {
    pub id: i32,
    pub canonical_title: String,
    pub cover_url: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub year: Option<i32>,
    /// Short synopsis. The list UI clamps this to a few lines; the detail
    /// page shows it in full.
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    /// Provenance of the row's metadata (`offline_cache`, `api`, or `manual`).
    /// The browse UI flags `manual` series so they read differently from
    /// provider-backed ones (no cover/metadata is expected).
    pub metadata_source: String,
    pub last_release_at: i64,
    pub first_seen_at: i64,
    /// Number of releases currently linked to this series. Surfaced as a
    /// badge in the feed so manual re-links that orphan a series (zero
    /// releases) are visible at a glance. Matches what
    /// `GET /releases?seriesId=…` returns.
    pub release_count: i64,
    /// Published total volume count from provider metadata; surfaced on
    /// the list view so sort-by-volume results have a number to display.
    pub total_volumes: Option<i32>,
    /// Published total chapter count from provider metadata; pair to
    /// [`Self::total_volumes`] for sort-by-chapter results.
    pub total_chapters: Option<i32>,
    /// Provider rating on a 0-10 scale; surfaced on the list view so a
    /// future sort-by-rating has a number to display.
    pub rating: Option<f64>,
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
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub metadata_source: String,
    pub metadata_fetched_at: i64,
    pub first_seen_at: i64,
    pub last_release_at: i64,
    pub highest_volume: Option<f64>,
    pub highest_chapter: Option<f64>,
    /// Published total volume count from provider metadata. Distinct from
    /// [`Self::highest_volume`], which tracks the highest span observed
    /// across releases. `None` if the provider has no value for this row.
    pub total_volumes: Option<i32>,
    /// Published total chapter count from provider metadata. See
    /// [`Self::total_volumes`] for how this differs from `highest_chapter`.
    pub total_chapters: Option<i32>,
    /// Provider rating on a 0-10 scale (normalized in the provider's
    /// mapping layer). `None` when the provider has no rating.
    pub rating: Option<f64>,
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
    /// Filter by whether any releases are linked to the series. `true`
    /// keeps only series with ≥1 release; `false` keeps only orphaned
    /// series (zero releases — often the residue of a manual re-link).
    pub has_releases: Option<bool>,
    /// Comma-separated genre names. Combined with [`Self::genres_mode`]:
    /// `any` (default) keeps series matching at least one; `all` keeps
    /// only series matching every entry. Each entry is matched case-
    /// insensitively. AND-combined with the other filters.
    pub genres: Option<String>,
    /// `any` (default) or `all`. See [`Self::genres`].
    pub genres_mode: Option<String>,
    /// Comma-separated tag names. Mirrors [`Self::genres`].
    pub tags: Option<String>,
    /// `any` (default) or `all`. See [`Self::tags`].
    pub tags_mode: Option<String>,
    /// Sort field. Supports `last_release_at` (default), `first_seen_at`,
    /// `total_volumes`, and `total_chapters`. The count sorts are
    /// nullable-aware: rows without a provider value sink to the end
    /// regardless of direction. Ignored when `q` is present (results
    /// are ranked by relevance instead).
    pub sort: Option<String>,
    /// `asc` or `desc` (default).
    pub order: Option<String>,
    /// Free-text query. When set, results are ranked by a Dice-coefficient
    /// score against canonical + alternate titles, with a boost for FTS5
    /// prefix matches. Whitespace-only is treated as absent.
    pub q: Option<String>,
}

impl Default for SeriesListQuery {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: 50,
            kind: None,
            status: None,
            owned: None,
            has_releases: None,
            genres: None,
            genres_mode: None,
            tags: None,
            tags_mode: None,
            sort: None,
            order: None,
            q: None,
        }
    }
}

/// Score floor for the Dice rerank. Hits below this are treated as
/// no-match (avoids returning random titles for nonsense queries).
const SEARCH_DICE_FLOOR: f32 = 0.30;
/// Upper bound on the in-memory Dice scan. Personal-scale catalogs are
/// nowhere near this; the cap exists so a degenerate dataset can't
/// accidentally hold the request thread for seconds.
const SEARCH_DICE_CANDIDATE_CAP: u64 = 5000;
/// Additive boost applied to a series' Dice score when FTS5 also matched
/// the query. Big enough to break ties confidently in favor of the
/// "user spelled it right" path without overriding a genuinely better
/// fuzzy match elsewhere.
const SEARCH_FTS_BOOST: f32 = 0.50;
/// FTS5 candidate-set size. 200 is plenty for personal scale and keeps
/// the boost set bounded.
const SEARCH_FTS_FETCH_LIMIT: u64 = 200;

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
    let q_text: Option<String> =
        q.q.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
    if let Some(q_raw) = q_text {
        return search_list(state, q, &q_raw).await;
    }

    let mut select = apply_series_filters(series::Entity::find(), &q);
    let sort_col = match q.sort.as_deref() {
        Some("first_seen_at") => series::Column::FirstSeenAt,
        Some("total_volumes") => series::Column::TotalVolumes,
        Some("total_chapters") => series::Column::TotalChapters,
        _ => series::Column::LastReleaseAt,
    };
    let desc = !matches!(q.order.as_deref(), Some("asc"));
    // Nullable count columns: prefix the order with `IS NULL ASC` so rows
    // without a provider value sink to the bottom for both directions —
    // otherwise SQLite puts NULLs first on DESC and a "longest first"
    // sort would lead with rows of unknown length. The default
    // `last_release_at` / `first_seen_at` columns are NOT NULL and need
    // no such prefix.
    if matches!(
        sort_col,
        series::Column::TotalVolumes | series::Column::TotalChapters
    ) {
        select = select.order_by_asc(Expr::col(sort_col).is_null());
    }
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

    let items = decorate_list_items(&state, rows).await?;
    Ok(Json(SeriesListPage {
        items,
        page: pagination.page(),
        page_size: pagination.page_size(),
        total,
    }))
}

/// Hydrate a page of series rows with their normalized genres + tags +
/// release counts via a single SELECT per relation.
async fn decorate_list_items(
    state: &AppState,
    rows: Vec<series::Model>,
) -> ApiResult<Vec<SeriesListItem>> {
    let ids: Vec<i32> = rows.iter().map(|m| m.id).collect();
    let genres_map = tagging_repo::genres_by_series_ids(&state.db, &ids)
        .await
        .map_err(anyhow_err)?;
    let tags_map = tagging_repo::tags_by_series_ids(&state.db, &ids)
        .await
        .map_err(anyhow_err)?;
    let counts_map = releases_repo::count_by_series_ids(&state.db, &ids)
        .await
        .map_err(anyhow_err)?;
    Ok(rows
        .into_iter()
        .map(|m| {
            let genres = genres_map.get(&m.id).cloned().unwrap_or_default();
            let tags = tags_map.get(&m.id).cloned().unwrap_or_default();
            let release_count = counts_map.get(&m.id).copied().unwrap_or(0);
            model_to_list_item(m, genres, tags, release_count)
        })
        .collect())
}

/// Apply the column-level and join-level filters shared by both the
/// no-query path and the search path. Pulled out so the search path can
/// re-apply them as a server-side intersection against its ranked
/// candidate set.
fn apply_series_filters(
    mut select: sea_orm::Select<series::Entity>,
    q: &SeriesListQuery,
) -> sea_orm::Select<series::Entity> {
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
    if let Some(has_releases) = q.has_releases {
        // Subquery rather than a JOIN so the outer count stays correct
        // even for series with many releases (a JOIN would multiply rows).
        // `series_id` on the releases table is NULL for unresolved rows,
        // so the IS NOT NULL guard is what makes "has releases" mean
        // "linked to this series specifically" and not "exists in the
        // releases table somewhere".
        let linked_ids = sea_orm::sea_query::Query::select()
            .column(releases::Column::SeriesId)
            .from(releases::Entity)
            .distinct()
            .and_where(releases::Column::SeriesId.is_not_null())
            .take();
        select = if has_releases {
            select.filter(series::Column::Id.in_subquery(linked_ids))
        } else {
            select.filter(series::Column::Id.not_in_subquery(linked_ids))
        };
    }
    // Genre / tag filters: semi-joins via a subquery so the outer SELECT
    // never row-multiplies (which would break pagination counts) and so
    // the `all` mode can use a GROUP BY ... HAVING COUNT on the join
    // table. Names are matched case-insensitively because the underlying
    // UNIQUE constraints collate NOCASE.
    let genre_names = parse_csv(q.genres.as_deref());
    if !genre_names.is_empty() {
        let sub = genre_semijoin_subquery(&genre_names, is_all_mode(q.genres_mode.as_deref()));
        select = select.filter(series::Column::Id.in_subquery(sub));
    }
    let tag_names = parse_csv(q.tags.as_deref());
    if !tag_names.is_empty() {
        let sub = tag_semijoin_subquery(&tag_names, is_all_mode(q.tags_mode.as_deref()));
        select = select.filter(series::Column::Id.in_subquery(sub));
    }
    select
}

/// Split a comma-separated genre / tag list into trimmed, non-empty names.
/// Returns an empty vec if the input is absent or yields no usable tokens —
/// the caller treats that as "no constraint" rather than "match nothing".
fn parse_csv(raw: Option<&str>) -> Vec<String> {
    let Some(s) = raw else { return Vec::new() };
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// `all` (case-insensitive) ⇒ every name must match; anything else ⇒ at
/// least one. Default is `any` so an absent mode behaves like the
/// pre-multiselect single-name filter.
fn is_all_mode(raw: Option<&str>) -> bool {
    matches!(
        raw.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("all")
    )
}

/// Subquery yielding the series ids that match the requested genres.
/// In `all` mode every requested name must match (`GROUP BY series_id
/// HAVING COUNT(DISTINCT genres.name) = N`); otherwise at least one is
/// enough. The select is duplicate-safe because the outer call always
/// uses it inside `series::Column::Id.in_subquery(...)`.
fn genre_semijoin_subquery(
    names: &[String],
    all_mode: bool,
) -> sea_orm::sea_query::SelectStatement {
    use sea_orm::QueryTrait;
    use sea_orm::sea_query::Func;

    let mut sub = series_genres::Entity::find()
        .select_only()
        .column(series_genres::Column::SeriesId)
        .join(
            sea_orm::JoinType::InnerJoin,
            series_genres::Relation::Genre.def(),
        )
        .filter(genres::Column::Name.is_in(names.iter().cloned()));
    if all_mode {
        sub = sub.group_by(series_genres::Column::SeriesId).having(
            Expr::expr(Func::count_distinct(Expr::col((
                genres::Entity,
                genres::Column::Name,
            ))))
            .eq(names.len() as i64),
        );
    }
    sub.into_query()
}

/// Tag analog of [`genre_semijoin_subquery`]. Separate function rather
/// than a generic helper because sea-orm's typing makes the abstraction
/// more fiddly than just writing the second variant.
fn tag_semijoin_subquery(names: &[String], all_mode: bool) -> sea_orm::sea_query::SelectStatement {
    use sea_orm::QueryTrait;
    use sea_orm::sea_query::Func;

    let mut sub = series_tags::Entity::find()
        .select_only()
        .column(series_tags::Column::SeriesId)
        .join(
            sea_orm::JoinType::InnerJoin,
            series_tags::Relation::Tag.def(),
        )
        .filter(tags::Column::Name.is_in(names.iter().cloned()));
    if all_mode {
        sub = sub.group_by(series_tags::Column::SeriesId).having(
            Expr::expr(Func::count_distinct(Expr::col((
                tags::Entity,
                tags::Column::Name,
            ))))
            .eq(names.len() as i64),
        );
    }
    sub.into_query()
}

/// Free-text search path. Ranks every series by a Dice-coefficient
/// score against the cleaned query, boosts FTS5 prefix matches, and
/// intersects the result with the user-supplied filters. `sort` /
/// `order` are intentionally ignored: search is always relevance-first.
async fn search_list(
    state: AppState,
    q: SeriesListQuery,
    q_raw: &str,
) -> ApiResult<Json<SeriesListPage>> {
    let pagination = q.pagination();

    // FTS5 boost set. An empty match expression (all-punctuation input,
    // for example) skips the FTS pass entirely; Dice still runs.
    let fts_expr = build_fts_match_expression(q_raw);
    let fts_ids: HashSet<i32> = if fts_expr.is_empty() {
        HashSet::new()
    } else {
        series_repo::search_fts(&state.db, &fts_expr, SEARCH_FTS_FETCH_LIMIT)
            .await
            .map_err(anyhow_err)?
            .iter()
            .map(|m| m.id)
            .collect()
    };

    let cleaned = state.query_builder.clean(q_raw);
    let cleaned_primary = cleaned.primary().to_string();

    // Score every row (capped). At personal scale the table is tiny;
    // the cap exists so a runaway catalog can't pin the request thread.
    let all_rows = series::Entity::find()
        .limit(SEARCH_DICE_CANDIDATE_CAP)
        .all(&state.db)
        .await
        .map_err(anyhow_err)?;

    let mut scored: Vec<(f32, series::Model)> = all_rows
        .into_iter()
        .map(|m| {
            let mut titles: Vec<String> = vec![m.canonical_title.clone()];
            if let Some(json) = m.alternate_titles_json.as_deref()
                && let Ok(alts) = serde_json::from_str::<Vec<String>>(json)
            {
                titles.extend(alts);
            }
            let mut score = best_dice(&cleaned_primary, titles.iter().map(|s| s.as_str()));
            if fts_ids.contains(&m.id) {
                score += SEARCH_FTS_BOOST;
            }
            (score, m)
        })
        .filter(|(s, _)| *s >= SEARCH_DICE_FLOOR)
        .collect();
    // Highest score first; ties break by recency so a deterministic
    // ordering survives equal scores.
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.last_release_at.cmp(&a.1.last_release_at))
    });

    // Intersect with the user filters via SQL. Using a single SELECT
    // keeps genre / tag joins on the server where they belong.
    let candidate_ids: Vec<i32> = scored.iter().map(|(_, m)| m.id).collect();
    let allowed: HashSet<i32> = if candidate_ids.is_empty() {
        HashSet::new()
    } else {
        let select = apply_series_filters(series::Entity::find(), &q)
            .filter(series::Column::Id.is_in(candidate_ids));
        let id_only = select
            .select_only()
            .column(series::Column::Id)
            .into_tuple::<i32>()
            .all(&state.db)
            .await
            .map_err(anyhow_err)?;
        id_only.into_iter().collect()
    };

    let final_rows: Vec<series::Model> = scored
        .into_iter()
        .filter_map(|(_, m)| {
            if allowed.contains(&m.id) {
                Some(m)
            } else {
                None
            }
        })
        .collect();
    let total = final_rows.len() as u64;
    let start = pagination.offset() as usize;
    let end = (start + pagination.limit() as usize).min(final_rows.len());
    let page_rows: Vec<series::Model> = if start >= final_rows.len() {
        Vec::new()
    } else {
        final_rows[start..end].to_vec()
    };
    let items = decorate_list_items(&state, page_rows).await?;
    Ok(Json(SeriesListPage {
        items,
        page: pagination.page(),
        page_size: pagination.page_size(),
        total,
    }))
}

/// Turn user-typed text into a safe FTS5 MATCH expression: alphanumeric
/// tokens only, each quoted, with a `*` suffix on the last token so a
/// partial last word still hits (e.g. "solo lev" matches "solo leveling").
/// Returns an empty string if no usable tokens remain.
fn build_fts_match_expression(raw: &str) -> String {
    let tokens: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();
    if tokens.is_empty() {
        return String::new();
    }
    let mut parts: Vec<String> = tokens.iter().map(|t| format!("\"{t}\"")).collect();
    let last = parts.len() - 1;
    parts[last].push('*');
    parts.join(" ")
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

/// Response from `POST /api/v1/series/refresh-all`. Mirrors the
/// triggered/skipped pattern used by the source and provider trigger
/// endpoints. `batchSize` echoes the config value the spawned tick will
/// use, so the operator can see at a glance how much work was queued.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefreshAllSeriesResponse {
    /// Active provider id the refresh ran against.
    pub provider: String,
    pub triggered: bool,
    /// `true` when a previous refresh tick is still in flight; the
    /// request is a no-op.
    pub skipped: bool,
    /// Maximum number of series rows this tick will touch, copied from
    /// `metadata.series_refresh.batch_size`. Reported even on
    /// `skipped: true` so the UI can render consistent metadata.
    pub batch_size: u32,
    /// Minimum age in days a row must have before it's eligible. Echoes
    /// `metadata.series_refresh.min_age_days`.
    pub min_age_days: u32,
}

/// Body for creating a manual series. Only `canonicalTitle` is required;
/// the rest are optional descriptive fields. No provider mapping is created
/// (that's the whole point), so the series is provider-agnostic and
/// `metadataSource` is pinned to `manual`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateSeriesRequest {
    pub canonical_title: String,
    pub kind: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub description: Option<String>,
}

/// Provenance marker for operator-authored series with no provider behind
/// them. Distinct from the resolver's `api` / `offline_cache` values so the
/// UI and any future per-series refresh can tell manual rows apart.
const MANUAL_METADATA_SOURCE: &str = "manual";

/// Create a manual series: a provider-less catalog entry for a real series
/// the active provider lacks. The operator then links releases to it via
/// `POST /releases/{id}/link` with `{ "seriesId": N }`.
///
/// No `series_external_ids` row is created, so this series never
/// participates in auto-resolution (the fuzzy resolver searches the
/// provider, not the local catalog) and is skipped by metadata refresh.
#[utoipa::path(
    post,
    path = "/api/v1/series",
    tag = "series",
    request_body = CreateSeriesRequest,
    responses(
        (status = 201, body = SeriesDetail),
        (status = 400, description = "canonicalTitle is empty")
    ),
    security(("admin" = []))
)]
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateSeriesRequest>,
) -> ApiResult<(StatusCode, Json<SeriesDetail>)> {
    let title = req.canonical_title.trim();
    if title.is_empty() {
        return Err(ApiError::BadRequest(
            "canonicalTitle must not be empty".into(),
        ));
    }
    let now = Utc::now().timestamp();
    let model = series::ActiveModel {
        canonical_title: Set(title.to_string()),
        alternate_titles_json: Set(None),
        cover_url: Set(req.cover_url.filter(|s| !s.trim().is_empty())),
        kind: Set(req.kind.filter(|s| !s.trim().is_empty())),
        status: Set(None),
        year: Set(req.year),
        description: Set(req.description.filter(|s| !s.trim().is_empty())),
        metadata_json: Set(None),
        metadata_source: Set(MANUAL_METADATA_SOURCE.into()),
        metadata_hash: Set(None),
        metadata_fetched_at: Set(now),
        first_seen_at: Set(now),
        last_release_at: Set(now),
        highest_volume: Set(None),
        highest_chapter: Set(None),
        owned: Set(0),
        ..Default::default()
    };
    let row = series_repo::create(&state.db, model)
        .await
        .map_err(anyhow_err)?;
    // A fresh manual series has no external ids, genres, or tags.
    Ok((
        StatusCode::CREATED,
        Json(model_to_detail(row, Vec::new(), Vec::new(), Vec::new())),
    ))
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

    // Explicit operator action: the per-series refresh button is the one
    // path that opts in to overwriting a manual row.
    let now = Utc::now();
    persist::upsert_series_from_metadata(
        &state.db,
        &active_id,
        &metadata,
        now.timestamp(),
        now,
        true,
    )
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

/// Trigger a bulk refresh of stale series rows against the active
/// metadata provider. The spawned tick reads `batch_size` and
/// `min_age_days` from `metadata.series_refresh`; the same selection
/// query backs the cron, so a manual click and a cron tick are
/// behaviourally identical (and they share a per-provider mutex, so
/// they can't race).
///
/// Returns immediately with `triggered: true` once the tick is spawned,
/// or `triggered: false, skipped: true` when a refresh is already in
/// flight for the active provider.
#[utoipa::path(
    post,
    path = "/api/v1/series/refresh-all",
    tag = "series",
    operation_id = "refresh_all_series",
    responses(
        (status = 202, body = RefreshAllSeriesResponse),
        (status = 503, description = "Active provider is not registered")
    ),
    security(("admin" = []))
)]
pub async fn refresh_all(
    State(state): State<AppState>,
) -> ApiResult<Json<RefreshAllSeriesResponse>> {
    let active_id = state.metadata.active_id().to_string();
    let provider = state.metadata.active().clone();
    let batch_size = state.metadata_config.series_refresh.batch_size;
    let min_age_days = state.metadata_config.series_refresh.min_age_days;
    let min_age_seconds = (min_age_days as i64).saturating_mul(86_400);

    let lock = state.locks.series_refresh_lock(&active_id);
    let db = state.db.clone();
    let events = state.job_events.clone();
    let started_at_ts = chrono::Utc::now().timestamp();
    let db_for_skip = state.db.clone();
    let id_for_skip = active_id.clone();
    let triggered = dispatch::try_dispatch(
        &state.job_events,
        lock,
        JobKind::SeriesRefresh,
        active_id.clone(),
        move || async move {
            refresh_series_metadata::record_skipped(
                &db_for_skip,
                &id_for_skip,
                started_at_ts,
                run_metrics_repo::trigger::MANUAL,
            )
            .await;
        },
        move || async move {
            refresh_series_metadata::run_tick(
                provider,
                db,
                batch_size,
                min_age_seconds,
                events,
                run_metrics_repo::trigger::MANUAL,
            )
            .await;
            JobResult {
                triggered: true,
                skipped: false,
                ..Default::default()
            }
        },
    );

    Ok(Json(RefreshAllSeriesResponse {
        provider: active_id,
        triggered,
        skipped: !triggered,
        batch_size,
        min_age_days,
    }))
}

/// Query string for `POST /api/v1/series/invalidate-metadata-hashes`.
/// `provider` scopes the operation to rows that have a `series_external_ids`
/// row for that provider; when omitted, every non-manual row is cleared.
#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct InvalidateMetadataHashesQuery {
    /// Provider id to scope the invalidation to (e.g. `"mangabaka"`).
    /// Optional. When absent, every provider-backed row is affected.
    pub provider: Option<String>,
}

/// Response from `POST /api/v1/series/invalidate-metadata-hashes`. The
/// endpoint runs synchronously, so the counts are the actual outcome of
/// the call (not "queued work").
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InvalidateMetadataHashesResponse {
    /// Echoes the `provider` query parameter, or `null` when the call
    /// was not scoped.
    pub provider: Option<String>,
    /// Number of `series` rows whose `metadata_hash` was cleared. A
    /// subsequent series-refresh tick will rewrite each one because the
    /// hash short-circuit no longer fires.
    pub invalidated: u32,
    /// Number of rows that matched the scope but were left untouched
    /// because `metadata_source = 'manual'`. Reported so the operator
    /// can spot when a meaningful chunk of the catalog is curated by
    /// hand (and therefore not eligible for upstream refresh).
    pub skipped_manual: u32,
}

/// Clear `metadata_hash` for every provider-backed series row in scope so
/// the next refresh tick rewrites them. The persist layer short-circuits
/// the series UPDATE when the incoming provider payload hashes to the
/// stored value; that's the right call for steady-state refreshes, but it
/// strands existing rows whenever a new denormalized column lands on the
/// `series` table (the upstream payload is unchanged → hash matches →
/// write skipped → new column stays NULL forever).
///
/// This endpoint is the operator escape hatch for that scenario. It runs
/// synchronously and returns the affected count; the operator then
/// triggers `/series/refresh-all` (or waits for the next cron tick) to
/// actually rewrite the rows.
///
/// Manual rows (`metadata_source = 'manual'`) are always left alone.
#[utoipa::path(
    post,
    path = "/api/v1/series/invalidate-metadata-hashes",
    tag = "series",
    operation_id = "invalidate_series_metadata_hashes",
    params(InvalidateMetadataHashesQuery),
    responses((status = 200, body = InvalidateMetadataHashesResponse)),
    security(("admin" = []))
)]
pub async fn invalidate_metadata_hashes(
    State(state): State<AppState>,
    Query(params): Query<InvalidateMetadataHashesQuery>,
) -> ApiResult<Json<InvalidateMetadataHashesResponse>> {
    let provider = params
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let outcome = series_repo::invalidate_metadata_hashes(&state.db, provider)
        .await
        .map_err(anyhow_err)?;
    Ok(Json(InvalidateMetadataHashesResponse {
        provider: provider.map(str::to_owned),
        invalidated: u32::try_from(outcome.invalidated).unwrap_or(u32::MAX),
        skipped_manual: u32::try_from(outcome.skipped_manual).unwrap_or(u32::MAX),
    }))
}

fn model_to_list_item(
    m: series::Model,
    genres: Vec<String>,
    tags: Vec<String>,
    release_count: i64,
) -> SeriesListItem {
    SeriesListItem {
        id: m.id,
        canonical_title: m.canonical_title,
        cover_url: m.cover_url,
        kind: m.kind,
        status: m.status,
        year: m.year,
        description: m.description,
        genres,
        tags,
        metadata_source: m.metadata_source,
        last_release_at: m.last_release_at,
        first_seen_at: m.first_seen_at,
        release_count,
        total_volumes: m.total_volumes,
        total_chapters: m.total_chapters,
        rating: m.rating,
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
    SeriesDetail {
        id: m.id,
        canonical_title: m.canonical_title,
        alternate_titles,
        cover_url: m.cover_url,
        kind: m.kind,
        status: m.status,
        year: m.year,
        description: m.description,
        genres: join_genres,
        tags: join_tags,
        metadata_source: m.metadata_source,
        metadata_fetched_at: m.metadata_fetched_at,
        first_seen_at: m.first_seen_at,
        last_release_at: m.last_release_at,
        highest_volume: m.highest_volume,
        highest_chapter: m.highest_chapter,
        total_volumes: m.total_volumes,
        total_chapters: m.total_chapters,
        rating: m.rating,
        owned: m.owned != 0,
        external_ids: mappings
            .into_iter()
            .map(|x| ExternalIdDto {
                provider: x.provider,
                external_id: x.external_id,
                fetched_at: x.fetched_at,
            })
            .collect(),
    }
}

fn anyhow_err<E: Into<anyhow::Error>>(e: E) -> ApiError {
    ApiError::Internal(e.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_match_quotes_tokens_and_prefixes_last() {
        assert_eq!(
            build_fts_match_expression("solo leveling"),
            "\"solo\" \"leveling\"*"
        );
    }

    #[test]
    fn fts_match_treats_punctuation_as_token_separator() {
        // Mixed punctuation, including a leading bracket that would
        // otherwise be a syntax error in a raw FTS5 expression.
        assert_eq!(
            build_fts_match_expression("[scanlator] Solo-Leveling!"),
            "\"scanlator\" \"Solo\" \"Leveling\"*"
        );
    }

    #[test]
    fn fts_match_returns_empty_when_no_alphanumeric_tokens() {
        assert_eq!(build_fts_match_expression("!!! ???"), "");
        assert_eq!(build_fts_match_expression(""), "");
    }

    #[test]
    fn fts_match_handles_single_token_with_prefix() {
        assert_eq!(build_fts_match_expression("naruto"), "\"naruto\"*");
    }
}
