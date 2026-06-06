//! Series read endpoints + manual `refresh-metadata` write.

use std::collections::{HashMap, HashSet};

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
use td_db::repos::{
    codex_link_repo, codex_status_repo, releases_repo, series_external_ids_repo, series_repo,
    tagging_repo,
};
use td_metadata::SeriesMetadata;
use td_metadata::scoring::best_dice;
use td_resolution::persist;
use td_scheduler::dispatch;
use td_scheduler::jobs::refresh_series_metadata;
use utoipa::{IntoParams, ToSchema};

use crate::auth::MaybeAdmin;
use crate::codex_presence::{CodexInfo, CodexStatus, build_codex_info, compute_status};
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
    /// Highest volume number seen across this series' linked releases (what
    /// is realistically available), as opposed to [`Self::total_volumes`]
    /// (the published total). Drives the `vol available/total` card badge
    /// and the `highest_volume` sort. `None` until a numbered release links.
    pub highest_volume: Option<f64>,
    /// Highest chapter number seen across linked releases. Pairs with
    /// [`Self::total_chapters`] for the `ch available/total` card badge.
    pub highest_chapter: Option<f64>,
    /// Provider rating on a 0-10 scale; surfaced on the list view so a
    /// future sort-by-rating has a number to display.
    pub rating: Option<f64>,
    /// Whether the operator owns this series on Codex. Derived from the
    /// presence of [`Self::codex`], so it is only ever `true` for admins.
    pub owned: bool,
    /// Codex presence overlay. Present **only** for admin-authenticated
    /// requests and **only** when the series is on Codex; the key is absent
    /// (not null) otherwise, so the public read tier never learns library
    /// contents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexInfo>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeriesListPage {
    pub items: Vec<SeriesListItem>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    /// Timestamp of the last successful Codex sweep, or `null`/absent when not
    /// admin or no sweep has succeeded. The UI suppresses all Codex badges
    /// while this is absent so a pre-first-sync admin doesn't see false
    /// "not owned" states. Admin-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_synced_at: Option<i64>,
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
    /// Whether the operator owns this series on Codex. Derived from the
    /// presence of [`Self::codex`]; only ever `true` for admins.
    pub owned: bool,
    pub external_ids: Vec<ExternalIdDto>,
    /// Codex presence overlay. Admin-only; absent for non-admins and for
    /// series not on Codex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexInfo>,
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
    /// Filter by metadata provenance: `manual` keeps only operator-authored
    /// rows (`metadata_source = 'manual'`); `auto` keeps only provider-backed
    /// rows (`api` or `offline_cache`). Any other value (or absence) applies
    /// no constraint, matching the lenient handling of the other filters.
    pub metadata_source: Option<String>,
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
    /// `total_volumes`, `total_chapters`, `highest_volume`,
    /// `highest_chapter`, and `rating`. The count / highest / rating sorts
    /// are nullable-aware: rows without a value sink to the end regardless
    /// of direction.
    /// Ignored when `q` is present (results are ranked by relevance instead).
    pub sort: Option<String>,
    /// `asc` or `desc` (default).
    pub order: Option<String>,
    /// Free-text query. When set, results are ranked by a Dice-coefficient
    /// score against canonical + alternate titles, with a boost for FTS5
    /// prefix matches. Whitespace-only is treated as absent.
    pub q: Option<String>,
    /// Comma-separated Codex presence statuses, OR-combined: `any` (on Codex),
    /// `missing` (not on Codex), `complete`, `behind`, `present`, or `ignored`
    /// (completion tracking turned off via `ignore_completion`). A series is
    /// kept if it matches *any* listed status, so e.g. `missing,behind` returns
    /// everything not on Codex plus the owned-but-behind titles. **Admin-only
    /// and enforced server-side**: for a non-admin request the param is ignored
    /// entirely, so it can't be used to probe library contents.
    pub codex_status: Option<String>,
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
            metadata_source: None,
            genres: None,
            genres_mode: None,
            tags: None,
            tags_mode: None,
            sort: None,
            order: None,
            q: None,
            codex_status: None,
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
    MaybeAdmin(is_admin): MaybeAdmin,
) -> ApiResult<Json<SeriesListPage>> {
    let pagination = q.pagination();
    let q_text: Option<String> =
        q.q.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
    if let Some(q_raw) = q_text {
        return search_list(state, q, &q_raw, is_admin).await;
    }

    let mut select = apply_series_filters(series::Entity::find(), &q);
    // Codex presence filter: admin-only and enforced here, so a non-admin
    // request with `codexStatus` set just gets the unfiltered feed.
    if let Some(filter) = codex_status_filter(&state, &q, is_admin).await? {
        select = apply_codex_id_filter(select, &filter);
    }
    let sort_col = match q.sort.as_deref() {
        Some("first_seen_at") => series::Column::FirstSeenAt,
        Some("total_volumes") => series::Column::TotalVolumes,
        Some("total_chapters") => series::Column::TotalChapters,
        Some("highest_volume") => series::Column::HighestVolume,
        Some("highest_chapter") => series::Column::HighestChapter,
        Some("rating") => series::Column::Rating,
        _ => series::Column::LastReleaseAt,
    };
    let desc = !matches!(q.order.as_deref(), Some("asc"));
    // Nullable count columns: prefix the order with `IS NULL ASC` so rows
    // without a value sink to the bottom for both directions — otherwise
    // SQLite puts NULLs first on DESC and a "longest first" sort would
    // lead with rows of unknown length. The default `last_release_at` /
    // `first_seen_at` columns are NOT NULL and need no such prefix.
    if matches!(
        sort_col,
        series::Column::TotalVolumes
            | series::Column::TotalChapters
            | series::Column::HighestVolume
            | series::Column::HighestChapter
            | series::Column::Rating
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

    let items = decorate_list_items(&state, rows, is_admin).await?;
    Ok(Json(SeriesListPage {
        items,
        page: pagination.page(),
        page_size: pagination.page_size(),
        total,
        codex_synced_at: codex_synced_at(&state, is_admin).await?,
    }))
}

/// Hydrate a page of series rows with their normalized genres + tags +
/// release counts via a single SELECT per relation.
async fn decorate_list_items(
    state: &AppState,
    rows: Vec<series::Model>,
    is_admin: bool,
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
    // Codex links for this page — only loaded for admins, so the field is
    // structurally absent for everyone else.
    let codex_map: HashMap<i32, codex_link_repo::Model> = if is_admin {
        codex_link_repo::get_for_series_ids(&state.db, &ids)
            .await
            .map_err(anyhow_err)?
            .into_iter()
            .map(|l| (l.series_id, l))
            .collect()
    } else {
        HashMap::new()
    };
    let base_url = state.codex.normalized_base_url();
    Ok(rows
        .into_iter()
        .map(|m| {
            let genres = genres_map.get(&m.id).cloned().unwrap_or_default();
            let tags = tags_map.get(&m.id).cloned().unwrap_or_default();
            let release_count = counts_map.get(&m.id).copied().unwrap_or(0);
            let codex = codex_map.get(&m.id).map(|l| {
                build_codex_info(
                    l,
                    m.ignore_completion,
                    m.highest_volume,
                    m.highest_chapter,
                    base_url.as_deref(),
                )
            });
            model_to_list_item(m, genres, tags, release_count, codex)
        })
        .collect())
}

/// Resolve the admin-only `codexStatus` query param (a comma-separated,
/// OR-combined status list) to a series-id constraint, or `None` when no
/// constraint should apply (not admin, param absent/empty, or only
/// unrecognized values — treated leniently as "no filter"). The status
/// comparison reuses [`compute_status`] so the filter and the per-row badge
/// can never disagree.
pub(crate) async fn codex_status_filter(
    state: &AppState,
    q: &SeriesListQuery,
    is_admin: bool,
) -> ApiResult<Option<CodexIdFilter>> {
    if !is_admin {
        return Ok(None);
    }
    // Multi-select: statuses are OR-combined. Recognized values only; anything
    // unrecognized is dropped (lenient, mirrors how unknown sort fields fall
    // back rather than erroring).
    let statuses = parse_csv(q.codex_status.as_deref());
    let want_any = statuses.iter().any(|s| s == "any");
    let want_missing = statuses.iter().any(|s| s == "missing");
    let sub_wants: Vec<CodexStatus> = statuses
        .iter()
        .filter_map(|s| match s.as_str() {
            "complete" => Some(CodexStatus::Complete),
            "behind" => Some(CodexStatus::Behind),
            "present" => Some(CodexStatus::Present),
            "ignored" => Some(CodexStatus::Ignored),
            _ => None,
        })
        .collect();
    if !want_any && !want_missing && sub_wants.is_empty() {
        return Ok(None);
    }

    let links = codex_link_repo::list_all(&state.db)
        .await
        .map_err(anyhow_err)?;
    let linked_ids: Vec<i32> = links.iter().map(|l| l.series_id).collect();

    // Linked series the selection keeps. `any` already covers every linked
    // series, so the per-status scan is moot when it's selected.
    let included_linked: Vec<i32> = if want_any {
        linked_ids.clone()
    } else if sub_wants.is_empty() {
        Vec::new()
    } else {
        let highs = series_highs_by_ids(state, &linked_ids).await?;
        links
            .iter()
            .filter(|l| {
                let (ign, hv, hc) = highs
                    .get(&l.series_id)
                    .copied()
                    .unwrap_or((false, None, None));
                let st = compute_status(ign, hv, hc, l.local_max_volume, l.local_max_chapter);
                sub_wants.contains(&st)
            })
            .map(|l| l.series_id)
            .collect()
    };

    if want_missing {
        // Result is (unlinked) OR (included linked). With `any` also selected
        // that's every series, so don't constrain at all.
        if want_any {
            return Ok(None);
        }
        // Keep everything except the linked rows the selection didn't include.
        let included: HashSet<i32> = included_linked.iter().copied().collect();
        let excluded: Vec<i32> = linked_ids
            .into_iter()
            .filter(|id| !included.contains(id))
            .collect();
        Ok(Some(CodexIdFilter::Exclude(excluded)))
    } else {
        Ok(Some(CodexIdFilter::Include(included_linked)))
    }
}

/// A series-id constraint derived from the codex status filter.
pub(crate) enum CodexIdFilter {
    /// Keep only these ids (the OR-union of the selected on-Codex statuses).
    Include(Vec<i32>),
    /// Keep everything except these ids (selection includes `missing`).
    Exclude(Vec<i32>),
}

pub(crate) fn apply_codex_id_filter(
    select: sea_orm::Select<series::Entity>,
    filter: &CodexIdFilter,
) -> sea_orm::Select<series::Entity> {
    match filter {
        CodexIdFilter::Include(ids) => select.filter(series::Column::Id.is_in(ids.iter().copied())),
        CodexIdFilter::Exclude(ids) => {
            select.filter(series::Column::Id.is_not_in(ids.iter().copied()))
        }
    }
}

/// Map of `series_id -> (ignore_completion, highest_volume, highest_chapter)`
/// for the given ids. Used by the codex status filter to compute each linked
/// series' status; the ignore flag is threaded through [`compute_status`] so
/// the filter and the per-row badge agree on what counts as `Ignored`.
async fn series_highs_by_ids(
    state: &AppState,
    ids: &[i32],
) -> ApiResult<HashMap<i32, (bool, Option<f64>, Option<f64>)>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = series::Entity::find()
        .select_only()
        .column(series::Column::Id)
        .column(series::Column::IgnoreCompletion)
        .column(series::Column::HighestVolume)
        .column(series::Column::HighestChapter)
        .filter(series::Column::Id.is_in(ids.iter().copied()))
        .into_tuple::<(i32, bool, Option<f64>, Option<f64>)>()
        .all(&state.db)
        .await
        .map_err(anyhow_err)?;
    Ok(rows
        .into_iter()
        .map(|(id, ign, hv, hc)| (id, (ign, hv, hc)))
        .collect())
}

/// The last successful Codex sweep timestamp, admin-only and only when the
/// integration is enabled. Drives the UI badge-suppression guard.
async fn codex_synced_at(state: &AppState, is_admin: bool) -> ApiResult<Option<i64>> {
    if !is_admin || !state.codex.enabled {
        return Ok(None);
    }
    Ok(codex_status_repo::get(&state.db)
        .await
        .map_err(anyhow_err)?
        .and_then(|r| r.last_success_at))
}

/// Apply the column-level and join-level filters shared by both the
/// no-query path and the search path. Pulled out so the search path can
/// re-apply them as a server-side intersection against its ranked
/// candidate set.
pub(crate) fn apply_series_filters(
    mut select: sea_orm::Select<series::Entity>,
    q: &SeriesListQuery,
) -> sea_orm::Select<series::Entity> {
    // Kind / status accept one or more comma-separated values
    // (e.g. `kind=manga,manhwa`), OR-combined via `IN`. A single value is the
    // common case (what the browse UI sends) and still works as a one-element
    // set; the catalog export's multi-selects send several.
    let kinds = parse_csv(q.kind.as_deref());
    if !kinds.is_empty() {
        select = select.filter(series::Column::Kind.is_in(kinds));
    }
    let statuses = parse_csv(q.status.as_deref());
    if !statuses.is_empty() {
        select = select.filter(series::Column::Status.is_in(statuses));
    }
    if let Some(owned) = q.owned {
        let flag = if owned { 1 } else { 0 };
        select = select.filter(series::Column::Owned.eq(flag));
    }
    // Manual/auto provenance filter. `auto` is "any provider-backed row",
    // expressed as `!= manual` so it stays correct as new provider sources
    // (beyond api / offline_cache) are added. Unrecognized values fall
    // through to no constraint.
    match q.metadata_source.as_deref() {
        Some("manual") => {
            select = select.filter(series::Column::MetadataSource.eq(MANUAL_METADATA_SOURCE));
        }
        Some("auto") => {
            select = select.filter(series::Column::MetadataSource.ne(MANUAL_METADATA_SOURCE));
        }
        _ => {}
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
pub(crate) fn parse_csv(raw: Option<&str>) -> Vec<String> {
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
    is_admin: bool,
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
        let mut select = apply_series_filters(series::Entity::find(), &q)
            .filter(series::Column::Id.is_in(candidate_ids));
        // Same admin-only codex status filter as the no-query path.
        if let Some(filter) = codex_status_filter(&state, &q, is_admin).await? {
            select = apply_codex_id_filter(select, &filter);
        }
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
    let items = decorate_list_items(&state, page_rows, is_admin).await?;
    Ok(Json(SeriesListPage {
        items,
        page: pagination.page(),
        page_size: pagination.page_size(),
        total,
        codex_synced_at: codex_synced_at(&state, is_admin).await?,
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
    MaybeAdmin(is_admin): MaybeAdmin,
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
    // Admin-only Codex overlay, same gating as the list endpoint.
    let codex = if is_admin {
        codex_link_repo::get(&state.db, id)
            .await
            .map_err(anyhow_err)?
            .map(|l| {
                build_codex_info(
                    &l,
                    row.ignore_completion,
                    row.highest_volume,
                    row.highest_chapter,
                    state.codex.normalized_base_url().as_deref(),
                )
            })
    } else {
        None
    };
    Ok(Json(model_to_detail(
        row,
        mappings,
        genres_for_series,
        tags_for_series,
        codex,
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
    // A fresh manual series has no external ids, genres, tags, or Codex link.
    Ok((
        StatusCode::CREATED,
        Json(model_to_detail(
            row,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )),
    ))
}

/// Body for editing a manual series. Mirrors [`CreateSeriesRequest`] plus
/// `status` and `alternateTitles`. Only `canonicalTitle` is required; every
/// other field is a full replacement of the stored value (empty/absent →
/// cleared). `alternateTitles` replaces the whole list; `[]` or omission
/// clears it.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSeriesRequest {
    pub canonical_title: String,
    #[serde(default)]
    pub alternate_titles: Option<Vec<String>>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub description: Option<String>,
}

/// Edit a manual series' descriptive fields. **Manual rows only**: a
/// provider-backed series (`metadataSource` ≠ `manual`) is owned by the
/// provider and would have any edit overwritten on the next metadata refresh,
/// so it is rejected with `409`. Provider/metadata/provenance columns are never
/// touched — only the operator-authored descriptive fields change.
#[utoipa::path(
    patch,
    path = "/api/v1/series/{id}",
    tag = "series",
    params(("id" = i32, Path, description = "Internal series id")),
    request_body = UpdateSeriesRequest,
    responses(
        (status = 200, body = SeriesDetail),
        (status = 400, description = "canonicalTitle is empty"),
        (status = 404, description = "No series with that id"),
        (status = 409, description = "Series is provider-backed and not editable")
    ),
    security(("admin" = []))
)]
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<UpdateSeriesRequest>,
) -> ApiResult<Json<SeriesDetail>> {
    let title = req.canonical_title.trim();
    if title.is_empty() {
        return Err(ApiError::BadRequest(
            "canonicalTitle must not be empty".into(),
        ));
    }
    // Trim each alternate title and drop empties so search never sees blank
    // entries; the whole list is a full replacement.
    let alternate_titles: Vec<String> = req
        .alternate_titles
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    let edit = series_repo::ManualSeriesEdit {
        canonical_title: title.to_string(),
        alternate_titles,
        kind: req.kind.filter(|s| !s.trim().is_empty()),
        status: req.status.filter(|s| !s.trim().is_empty()),
        year: req.year,
        cover_url: req.cover_url.filter(|s| !s.trim().is_empty()),
        description: req.description.filter(|s| !s.trim().is_empty()),
    };

    let row = match series_repo::update_manual_fields(&state.db, id, edit)
        .await
        .map_err(anyhow_err)?
    {
        series_repo::UpdateManualOutcome::Updated(row) => *row,
        series_repo::UpdateManualOutcome::NotFound => {
            return Err(ApiError::NotFound(format!("series {id}")));
        }
        series_repo::UpdateManualOutcome::NotManual => {
            return Err(ApiError::Conflict(format!(
                "series {id} is provider-backed; only manual series can be edited"
            )));
        }
    };

    // A manual series has no external ids, genres, tags, or Codex link — but
    // re-hydrate via the same helpers as the read path so the response shape is
    // identical to GET and stays correct if a manual row ever gains a join row.
    let mappings = series_external_ids_repo::list_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    let tags_for_series = tagging_repo::list_tags_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    let genres_for_series = tagging_repo::list_genres_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    // This endpoint is admin-gated by the router, so surface the Codex overlay
    // like the read path does for an admin caller.
    let codex = codex_link_repo::get(&state.db, id)
        .await
        .map_err(anyhow_err)?
        .map(|l| {
            build_codex_info(
                &l,
                row.ignore_completion,
                row.highest_volume,
                row.highest_chapter,
                state.codex.normalized_base_url().as_deref(),
            )
        });
    Ok(Json(model_to_detail(
        row,
        mappings,
        genres_for_series,
        tags_for_series,
        codex,
    )))
}

/// Body for `PUT /api/v1/series/{id}/ignore-completion`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetIgnoreCompletionRequest {
    /// `true` to mute Codex completion tracking for this series (its Codex
    /// status becomes `ignored`), `false` to resume tracking.
    pub ignore: bool,
}

/// Toggle a series' `ignore_completion` flag. When set, the series' Codex
/// status is forced to `ignored` so the perpetually-false "Behind" signal is
/// muted — meant for series read in omnibus, where source single-volume
/// numbering is permanently ahead of the owned omnibus numbering.
///
/// Unlike the manual-edit `PATCH`, this works on **any** series (provider-backed
/// or manual): the flag is operator-owned and a metadata refresh never touches
/// it, so there is nothing to clobber. The series stays owned; only its tracked
/// status changes.
#[utoipa::path(
    put,
    path = "/api/v1/series/{id}/ignore-completion",
    tag = "series",
    params(("id" = i32, Path, description = "Internal series id")),
    request_body = SetIgnoreCompletionRequest,
    responses(
        (status = 200, body = SeriesDetail),
        (status = 404, description = "No series with that id")
    ),
    security(("admin" = []))
)]
pub async fn set_ignore_completion(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<SetIgnoreCompletionRequest>,
) -> ApiResult<Json<SeriesDetail>> {
    let row = match series_repo::set_ignore_completion(&state.db, id, req.ignore)
        .await
        .map_err(anyhow_err)?
    {
        series_repo::SetIgnoreCompletionOutcome::Updated(row) => *row,
        series_repo::SetIgnoreCompletionOutcome::NotFound => {
            return Err(ApiError::NotFound(format!("series {id}")));
        }
    };

    // Re-hydrate via the same helpers as the read path so the response shape
    // is identical to GET.
    let mappings = series_external_ids_repo::list_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    let tags_for_series = tagging_repo::list_tags_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    let genres_for_series = tagging_repo::list_genres_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    // Admin-gated by the router, so surface the Codex overlay like the read
    // path does — now reflecting the just-written flag.
    let codex = codex_link_repo::get(&state.db, id)
        .await
        .map_err(anyhow_err)?
        .map(|l| {
            build_codex_info(
                &l,
                row.ignore_completion,
                row.highest_volume,
                row.highest_chapter,
                state.codex.normalized_base_url().as_deref(),
            )
        });
    Ok(Json(model_to_detail(
        row,
        mappings,
        genres_for_series,
        tags_for_series,
        codex,
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
    // This endpoint is admin-gated by the router, so the caller is always an
    // admin — surface the Codex overlay like the read path does.
    let codex = codex_link_repo::get(&state.db, id)
        .await
        .map_err(anyhow_err)?
        .map(|l| {
            build_codex_info(
                &l,
                row.ignore_completion,
                row.highest_volume,
                row.highest_chapter,
                state.codex.normalized_base_url().as_deref(),
            )
        });
    Ok(Json(model_to_detail(
        row,
        mappings,
        genres_for_series,
        tags_for_series,
        codex,
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

/// Response from `POST /api/v1/series/recompute-spans`. The endpoint runs
/// synchronously, so the counts are the actual outcome of the pass.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecomputeSpansResponse {
    /// Releases whose stored volume/chapter span was rewritten because
    /// re-parsing the file list (or title) produced a different value.
    pub releases_rewritten: u64,
    /// Series rows whose `highest_volume` / `highest_chapter` changed.
    pub series_updated: u64,
}

/// Recompute every release's volume/chapter span and every series'
/// `highest_volume` / `highest_chapter` mark from the stored file lists
/// (titles as fallback). Network-free and idempotent. Unlike the
/// incremental bump that runs when a release is linked, this is an
/// authoritative pass: a series' marks are *replaced* with the MAX across
/// its currently-linked releases, so it also corrects values an earlier,
/// more eager parser over-counted and clears marks on series whose releases
/// no longer parse to anything.
///
/// Use it after changing the span-parsing logic, or to backfill a catalog
/// whose releases predate span detection. Runs in-process against the same
/// pool `serve` uses, so it is safe to trigger on a live instance (it shares
/// the single SQLite writer rather than contending with a separate process).
#[utoipa::path(
    post,
    path = "/api/v1/series/recompute-spans",
    tag = "series",
    operation_id = "recompute_series_spans",
    responses((status = 200, body = RecomputeSpansResponse)),
    security(("admin" = []))
)]
pub async fn recompute_spans(
    State(state): State<AppState>,
) -> ApiResult<Json<RecomputeSpansResponse>> {
    let summary = releases_repo::recompute_all_spans(&state.db)
        .await
        .map_err(anyhow_err)?;
    Ok(Json(RecomputeSpansResponse {
        releases_rewritten: summary.releases_rewritten,
        series_updated: summary.series_updated,
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
    codex: Option<CodexInfo>,
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
        highest_volume: m.highest_volume,
        highest_chapter: m.highest_chapter,
        rating: m.rating,
        // `owned` now reflects Codex ownership, surfaced only to admins (the
        // legacy `series.owned` column was never populated).
        owned: codex.is_some(),
        codex,
    }
}

fn model_to_detail(
    m: series::Model,
    mappings: Vec<series_external_ids::Model>,
    join_genres: Vec<String>,
    join_tags: Vec<String>,
    codex: Option<CodexInfo>,
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
        owned: codex.is_some(),
        external_ids: mappings
            .into_iter()
            .map(|x| ExternalIdDto {
                provider: x.provider,
                external_id: x.external_id,
                fetched_at: x.fetched_at,
            })
            .collect(),
        codex,
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
