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
    /// Official publication start/end dates from provider metadata, ISO
    /// `YYYY-MM-DD` (nullable). `publishedStartDate` backs the "Publication
    /// date" sort; distinct from [`Self::last_release_at`] (last *discovered*
    /// release).
    pub published_start_date: Option<String>,
    pub published_end_date: Option<String>,
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
    /// When tsundoku last discovered a release for this series, as opposed to
    /// [`Self::last_release_at`] (the newest linked release's *upstream* post
    /// date). `None` until something links. Backs the "Recently discovered"
    /// sort, which exists because a series found today from a year-old post
    /// sorts a year deep under `last_release_at`.
    pub last_discovered_at: Option<i64>,
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
    /// Whether the series is on the operator's wishlist. Admin-only: always
    /// `false` for non-admin requests (the curation list never reaches the
    /// public read tier), exactly like [`Self::owned`].
    pub wishlisted: bool,
    /// Epoch seconds the series was clipped to the wishlist, or `null` when not
    /// wishlisted / not admin. Drives the wishlist view's "recently clipped"
    /// sort.
    pub wishlisted_at: Option<i64>,
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

/// One inclusive `[start, end]` coverage range. Mirrors Codex's `NumericSpan`
/// (single values are `start == end`) so a release plugin can ingest the
/// `volumeCoverage` / `chapterCoverage` lists verbatim.
#[derive(Debug, Serialize, ToSchema)]
pub struct CoverageSpanDto {
    pub start: f64,
    pub end: f64,
}

/// One series in the incremental release feed (`GET /series/feed`). Carries
/// the provider IDs a consumer matches on plus the merged, gap-preserving
/// volume/chapter coverage across the series' linked releases.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeriesFeedItem {
    pub series_id: i32,
    pub canonical_title: String,
    /// Provider mappings (`provider` + `externalId`) — the match key for a
    /// consumer that keeps its own catalog keyed on, e.g., MangaBaka ids.
    pub external_ids: Vec<ExternalIdDto>,
    /// Merged available volume ranges (sorted, gaps preserved).
    pub volume_coverage: Vec<CoverageSpanDto>,
    /// Merged available chapter ranges (sorted, gaps preserved).
    pub chapter_coverage: Vec<CoverageSpanDto>,
    /// Max end of `volumeCoverage`, denormalized for a quick "behind?" check.
    pub highest_volume: Option<f64>,
    /// Max end of `chapterCoverage`.
    pub highest_chapter: Option<f64>,
    /// Epoch seconds this series' coverage last changed (the cursor key).
    pub updated_at: i64,
}

/// One page of the release feed. Walk while `hasMore` is true, passing
/// `nextCursor` back as `cursor`; when `hasMore` is false, keep `nextCursor`
/// as the bookmark and idle until the next poll.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeriesFeedResponse {
    pub items: Vec<SeriesFeedItem>,
    /// Opaque cursor at the last item returned, or absent when the page is
    /// empty. Treat it as a token — do not parse its structure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// `true` when more series remain after this page (fetch again now).
    pub has_more: bool,
}

const FEED_DEFAULT_LIMIT: u32 = 100;
const FEED_MAX_LIMIT: u32 = 500;

#[derive(Debug, Deserialize, IntoParams)]
#[serde(default, rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct SeriesFeedQuery {
    /// Opaque cursor from a previous response's `nextCursor`. Omit to start
    /// from the beginning.
    pub cursor: Option<String>,
    /// Max items per page (default 100, capped at 500).
    pub limit: u32,
}

impl Default for SeriesFeedQuery {
    fn default() -> Self {
        Self {
            cursor: None,
            limit: FEED_DEFAULT_LIMIT,
        }
    }
}

/// Body for `POST /series/feed`: the cursor + limit of the `GET`, plus the
/// provider id set to narrow the page to. It's a `POST` only so this list can
/// be large (a consumer's whole owned catalog) without hitting URL limits.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct SeriesFeedRequest {
    /// Opaque cursor from a previous response's `nextCursor`. Omit to start
    /// from the beginning.
    pub cursor: Option<String>,
    /// Max items per page (default 100, capped at 500).
    pub limit: Option<u32>,
    /// Narrow the page to series carrying one of these external-id mappings.
    ///
    /// Each entry is `provider:externalId` — the provider token, a colon, then
    /// the id (e.g. `mangabaka:12345`, `mangaupdates:234`). Matching is **OR**:
    /// a series is included if it carries *any* listed mapping, and is returned
    /// at most once. The `provider` half must match **exactly** (lowercase, no
    /// aliases). The full set of tokens tsundoku stores is: `mangabaka`,
    /// `mangaupdates`, `mal`, `anilist`, `mangadex`, `kitsu`, `anime_planet`,
    /// `anime_news_network`, `shikimori`. Prefer round-tripping the `provider`
    /// value from a feed item's `externalIds` verbatim over guessing
    /// (`mangaupdates:234`, not `mu:234`). Entries without a colon are ignored.
    /// Empty ⇒ no filter (identical to the `GET`).
    pub external_ids: Vec<String>,
}

/// Encode a keyset position into an opaque cursor token.
fn encode_feed_cursor(updated_at: i64, id: i32) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(format!("{updated_at}:{id}"))
}

/// Decode a cursor token back into `(updated_at, id)`. `None` on any
/// malformed input so the handler can return a 400 rather than silently
/// restarting the walk.
fn decode_feed_cursor(token: &str) -> Option<(i64, i32)> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let bytes = URL_SAFE_NO_PAD.decode(token).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let (updated_at, id) = text.split_once(':')?;
    Some((updated_at.parse().ok()?, id.parse().ok()?))
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
    /// Official publication start/end dates from provider metadata, ISO
    /// `YYYY-MM-DD` (nullable). See [`SeriesListItem::published_start_date`].
    pub published_start_date: Option<String>,
    pub published_end_date: Option<String>,
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub metadata_source: String,
    pub metadata_fetched_at: i64,
    pub first_seen_at: i64,
    pub last_release_at: i64,
    /// See [`SeriesListItem::last_discovered_at`].
    pub last_discovered_at: Option<i64>,
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
    /// Whether the series is on the operator's wishlist. Admin-only (always
    /// `false` for non-admin requests), like [`Self::owned`].
    pub wishlisted: bool,
    /// Epoch seconds the series was clipped to the wishlist, or `null` when not
    /// wishlisted / not admin.
    pub wishlisted_at: Option<i64>,
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
    /// Filter by wishlist flag: `true` keeps only wishlisted series, `false`
    /// only non-wishlisted, absent applies no constraint. **Admin-only and
    /// enforced server-side**: ignored entirely for a non-admin request, like
    /// [`Self::codex_status`], so the curation list can't be probed.
    pub wishlisted: Option<bool>,
    /// Filter by whether any releases are linked to the series. `true`
    /// keeps only series with ≥1 release; `false` keeps only orphaned
    /// series (zero releases — often the residue of a manual re-link).
    pub has_releases: Option<bool>,
    /// Comma-separated discovery-source names (the release `source_name`,
    /// e.g. `english-manga-trusted`), OR-combined: a series is kept if it
    /// has ≥1 linked release from *any* listed source. **Admin-only and
    /// enforced server-side**: ignored entirely for a non-admin request,
    /// like [`Self::codex_status`], so the curated narrowing can't be probed.
    pub source: Option<String>,
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
    /// `last_discovered_at` (when tsundoku last *found* a release, as opposed
    /// to when it was posted upstream), `total_volumes`, `total_chapters`,
    /// `highest_volume`, `highest_chapter`, `rating`, `published_start_date`
    /// (official publication date, distinct from `last_release_at`), and
    /// `wishlisted_at` (admin "recently clipped" order for the wishlist view).
    /// The count / highest / rating / publication / wishlisted / discovered
    /// sorts are nullable-aware: rows without a value sink to the end
    /// regardless of direction.
    /// Ignored when `q` is present (results are ranked by relevance instead).
    pub sort: Option<String>,
    /// `asc` or `desc` (default).
    pub order: Option<String>,
    /// Free-text query. When set, results are ranked by a Dice-coefficient
    /// score against canonical + alternate titles, with a boost for FTS5
    /// prefix matches. Whitespace-only is treated as absent.
    pub q: Option<String>,
    /// When `true`, the free-text [`Self::q`] also matches series descriptions,
    /// not just titles. Defaults to `false` (titles only). Only meaningful
    /// alongside `q`; ignored when `q` is absent. Description-only matches rank
    /// below genuine title matches (the relevance score is title-based).
    pub search_descriptions: Option<bool>,
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
            wishlisted: None,
            has_releases: None,
            source: None,
            metadata_source: None,
            genres: None,
            genres_mode: None,
            tags: None,
            tags_mode: None,
            sort: None,
            order: None,
            q: None,
            search_descriptions: None,
            codex_status: None,
        }
    }
}

/// Score floor for the Dice rerank. Hits below this are treated as
/// no-match (avoids returning random titles for nonsense queries).
const SEARCH_DICE_FLOOR: f32 = 0.30;
/// Catalog size past which the in-memory Dice scan is worth revisiting. This
/// is a *log* threshold, not a cap: the scan stays exhaustive above it.
///
/// A hard `LIMIT` used to sit here instead, which silently dropped every row
/// past it — and because the query had no `ORDER BY`, SQLite scanned in rowid
/// order, so what got dropped was always the most recently added series. A
/// search that cannot find new titles is worse than a slow one, and the scan
/// is measured in single-digit milliseconds at personal scale anyway.
const SEARCH_SCAN_WARN_ROWS: usize = 50_000;
/// Additive boost applied to a series' Dice score when FTS5 also matched
/// the query. Big enough to break ties confidently in favor of the
/// "user spelled it right" path without overriding a genuinely better
/// fuzzy match elsewhere.
const SEARCH_FTS_BOOST: f32 = 0.50;
// The FTS pass is deliberately unbounded — see
// `series_repo::search_fts_ids`. It used to fetch a top-200 slice, which
// dropped over half the match set for common tokens and, because an FTS hit
// bypasses the Dice floor, silently lost those candidates entirely.

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

    let mut select = apply_series_filters(series::Entity::find(), &q, is_admin);
    // Codex presence filter: admin-only and enforced here, so a non-admin
    // request with `codexStatus` set just gets the unfiltered feed.
    if let Some(filter) = codex_status_filter(&state, &q, is_admin).await? {
        select = apply_codex_id_filter(select, &filter);
    }
    let sort_col = match q.sort.as_deref() {
        Some("first_seen_at") => series::Column::FirstSeenAt,
        // When tsundoku last *found* a release, not when it was posted
        // upstream. The two diverge by months whenever a source surfaces an
        // old post (query feeds, backfills, the per-series release search),
        // which is what buries a fresh discovery under `last_release_at`.
        // Nullable (NULL = nothing linked), so it joins the NULL-last set.
        Some("last_discovered_at") => series::Column::LastDiscoveredAt,
        Some("total_volumes") => series::Column::TotalVolumes,
        Some("total_chapters") => series::Column::TotalChapters,
        Some("highest_volume") => series::Column::HighestVolume,
        Some("highest_chapter") => series::Column::HighestChapter,
        Some("rating") => series::Column::Rating,
        // Official publication start date (ISO `YYYY-MM-DD` TEXT, lexicographic
        // order == chronological). Nullable, so it joins the NULL-last set.
        Some("published_start_date") => series::Column::PublishedStartDate,
        // Admin-only "recently clipped" order for the wishlist view. The column
        // is nullable (NULL = not wishlisted), so it joins the nullable-aware
        // ordering below; the wishlist view also filters `wishlisted=true`, so
        // in practice every row it sees has a value.
        Some("wishlisted_at") => series::Column::WishlistedAt,
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
            | series::Column::WishlistedAt
            | series::Column::PublishedStartDate
            | series::Column::LastDiscoveredAt
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
            model_to_list_item(m, genres, tags, release_count, codex, is_admin)
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
    is_admin: bool,
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
    // Wishlist filter: admin-only operator curation, so a non-admin request
    // never narrows by it (the field is also blanked in the DTO). `true` keeps
    // rows with a clip timestamp, `false` keeps those without.
    if is_admin && let Some(wishlisted) = q.wishlisted {
        select = if wishlisted {
            select.filter(series::Column::WishlistedAt.is_not_null())
        } else {
            select.filter(series::Column::WishlistedAt.is_null())
        };
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
    // Source filter: admin-only operator curation (a non-admin request never
    // narrows by it, mirroring the wishlist / Codex filters). Same semi-join
    // shape as `has_releases` above — a subquery rather than a JOIN keeps the
    // outer pagination count correct — with the source-name predicate added.
    let source_names = parse_csv(q.source.as_deref());
    if is_admin && !source_names.is_empty() {
        let linked_ids = sea_orm::sea_query::Query::select()
            .column(releases::Column::SeriesId)
            .from(releases::Entity)
            .distinct()
            .and_where(releases::Column::SeriesId.is_not_null())
            .and_where(releases::Column::SourceName.is_in(source_names))
            .take();
        select = select.filter(series::Column::Id.in_subquery(linked_ids));
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

    // FTS5 match set: a ranking boost *and* a floor bypass, so it has to be
    // the whole set rather than a top-N slice. An empty match expression
    // (all-punctuation input, for example) skips the FTS pass entirely; Dice
    // still runs.
    let fts_expr = build_fts_match_expression(q_raw, q.search_descriptions.unwrap_or(false));
    let fts_ids: HashSet<i32> = if fts_expr.is_empty() {
        HashSet::new()
    } else {
        series_repo::search_fts_ids(&state.db, &fts_expr)
            .await
            .map_err(anyhow_err)?
            .into_iter()
            .collect()
    };

    let cleaned = state.query_builder.clean(q_raw);
    let cleaned_primary = cleaned.primary().to_string();

    // Score every row — no cap. A `LIMIT` here without an `ORDER BY` made
    // SQLite stop at whatever it reached in rowid order, which silently made
    // the *newest* series unfindable and got worse with every row added.
    //
    // Only the scoring columns are selected. Full models would drag
    // `metadata_json` and `description` through every search for data the
    // scorer never reads — on a 5k-row catalog that is ~16MB of payload to
    // rank against ~1.6MB of titles.
    let candidates: Vec<(i32, String, Option<String>, i64)> = series::Entity::find()
        .select_only()
        .column(series::Column::Id)
        .column(series::Column::CanonicalTitle)
        .column(series::Column::AlternateTitlesJson)
        .column(series::Column::LastReleaseAt)
        .into_tuple()
        .all(&state.db)
        .await
        .map_err(anyhow_err)?;
    if candidates.len() > SEARCH_SCAN_WARN_ROWS {
        tracing::warn!(
            rows = candidates.len(),
            threshold = SEARCH_SCAN_WARN_ROWS,
            "series catalog is large enough that the in-memory search scan is worth revisiting; \
             scoring is still exhaustive (never truncated)"
        );
    }

    let mut scored: Vec<(f32, i32, i64)> = candidates
        .into_iter()
        .filter_map(
            |(id, canonical_title, alternate_titles_json, last_release_at)| {
                let mut titles: Vec<String> = vec![canonical_title];
                if let Some(json) = alternate_titles_json.as_deref()
                    && let Ok(alts) = serde_json::from_str::<Vec<String>>(json)
                {
                    titles.extend(alts);
                }
                let is_fts_hit = fts_ids.contains(&id);
                let mut score = best_dice(&cleaned_primary, titles.iter().map(|s| s.as_str()));
                if is_fts_hit {
                    score += SEARCH_FTS_BOOST;
                }
                // An FTS5 hit is a match whatever Dice thinks of it — a one-word
                // query against a long title scores far below the floor. The boost
                // currently clears the floor on its own, but stating the rule
                // explicitly stops the two constants from being silently coupled.
                (is_fts_hit || score >= SEARCH_DICE_FLOOR).then_some((score, id, last_release_at))
            },
        )
        .collect();
    // Highest score first; ties break by recency so a deterministic
    // ordering survives equal scores.
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.2.cmp(&a.2))
    });

    // Intersect with the user filters via SQL. Using a single SELECT
    // keeps genre / tag joins on the server where they belong.
    let candidate_ids: Vec<i32> = scored.iter().map(|(_, id, _)| *id).collect();
    let allowed: HashSet<i32> = if candidate_ids.is_empty() {
        HashSet::new()
    } else {
        let mut select = apply_series_filters(series::Entity::find(), &q, is_admin)
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

    let final_ids: Vec<i32> = scored
        .into_iter()
        .filter_map(|(_, id, _)| allowed.contains(&id).then_some(id))
        .collect();
    let total = final_ids.len() as u64;
    let start = pagination.offset() as usize;
    let end = (start + pagination.limit() as usize).min(final_ids.len());
    let page_ids: &[i32] = if start >= final_ids.len() {
        &[]
    } else {
        &final_ids[start..end]
    };
    // Now — and only now — pull the full rows, for the one page being
    // returned rather than the whole catalog. `is_in` loses the ranked order,
    // so reindex by id to restore it.
    let mut by_id: HashMap<i32, series::Model> = if page_ids.is_empty() {
        HashMap::new()
    } else {
        series::Entity::find()
            .filter(series::Column::Id.is_in(page_ids.iter().copied()))
            .all(&state.db)
            .await
            .map_err(anyhow_err)?
            .into_iter()
            .map(|m| (m.id, m))
            .collect()
    };
    let page_rows: Vec<series::Model> = page_ids.iter().filter_map(|id| by_id.remove(id)).collect();
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
///
/// When `include_description` is false (the default search mode), the whole
/// expression is wrapped in a `{title alternate_titles} : (…)` column filter
/// so matching is pinned to titles — identical to the behavior from before
/// `description` was added to the FTS table. The parentheses are required:
/// the column-filter operator binds tighter than the implicit AND, so without
/// them only the first token would be scoped. When true, the unfiltered form
/// is emitted, which also spans the `description` column.
fn build_fts_match_expression(raw: &str, include_description: bool) -> String {
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
    let joined = parts.join(" ");
    if include_description {
        joined
    } else {
        format!("{{title alternate_titles}} : ({joined})")
    }
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
        is_admin,
    )))
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct SeriesLookupParams {
    /// Provider token as stored in `series_external_ids.provider`
    /// (e.g. `mangabaka`, `mal`, `anime_planet`). Case-insensitive.
    ///
    /// Optional. With it the lookup is provider-qualified and returns at most
    /// one match, guaranteed by the `UNIQUE(provider, external_id)`
    /// constraint. Without it every provider is searched, which can return
    /// several: id spaces overlap, so the same number is a different series on
    /// MAL than on MangaBaka. Ignored when [`Self::external_id`] is a
    /// recognized series URL, since the URL already names its provider.
    pub provider: Option<String>,
    /// The provider's own id for the series, matched exactly (after trim), or
    /// a full series URL (MangaBaka, AniList, MyAnimeList, MangaDex,
    /// MangaUpdates) to infer the provider from.
    pub external_id: String,
}

/// One `(provider, externalId) → series` hit. Carries the title so a caller
/// disambiguating several matches has something to show the user.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeriesLookupMatch {
    pub series_id: i32,
    pub provider: String,
    pub external_id: String,
    pub canonical_title: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeriesLookupResponse {
    /// Matching series, ordered by provider. Empty when nothing maps — a
    /// normal outcome (tsundoku only knows series it has discovered), not an
    /// error, so this is a `200` rather than a `404`.
    pub matches: Vec<SeriesLookupMatch>,
}

/// Resolve an external id — bare, provider-qualified, or a pasted series URL —
/// to the series that carries it.
///
/// Backs two callers: the `/series/lookup` deep-link page that external tools
/// (Codex's plugin web-links button) point at, and the in-app lookup modal.
///
/// This is a key lookup, not a search: with a provider it is 0-or-1 by
/// schema constraint. A bare id has no such guarantee, so the response is a
/// list and disambiguation is the caller's job.
#[utoipa::path(
    get,
    path = "/api/v1/series/lookup",
    tag = "series",
    params(SeriesLookupParams),
    responses((status = 200, body = SeriesLookupResponse))
)]
pub async fn lookup(
    State(state): State<AppState>,
    Query(params): Query<SeriesLookupParams>,
) -> ApiResult<Json<SeriesLookupResponse>> {
    let raw = params.external_id.trim();
    if raw.is_empty() {
        return Ok(Json(SeriesLookupResponse { matches: vec![] }));
    }

    // A recognized URL names its own provider, so it outranks the `provider`
    // param rather than being second-guessed by it.
    let resolved = match td_resolution::foreign_id::detect(raw) {
        Some((provider, id)) => resolve_detected_pair(&state, provider, id).await?,
        None => match params.provider.as_deref().map(str::trim) {
            Some(p) if !p.is_empty() => Some((p.to_ascii_lowercase(), raw.to_string())),
            // Bare id, no provider: fall through to the all-provider search.
            _ => None,
        },
    };

    let rows = match resolved {
        Some((provider, external_id)) => {
            series_external_ids_repo::find_series_id(&state.db, &provider, &external_id)
                .await
                .map_err(anyhow_err)?
                .map(|series_id| series_external_ids_repo::Model {
                    provider,
                    external_id,
                    series_id,
                    fetched_at: 0,
                })
                .into_iter()
                .collect()
        }
        None => series_external_ids_repo::find_by_external_id(&state.db, raw)
            .await
            .map_err(anyhow_err)?,
    };

    // Titles for the disambiguation list. One query for the whole (tiny) set.
    let series_ids: Vec<i32> = rows.iter().map(|r| r.series_id).collect();
    let titles: HashMap<i32, String> = if series_ids.is_empty() {
        HashMap::new()
    } else {
        series::Entity::find()
            .filter(series::Column::Id.is_in(series_ids))
            .select_only()
            .column(series::Column::Id)
            .column(series::Column::CanonicalTitle)
            .into_tuple::<(i32, String)>()
            .all(&state.db)
            .await
            .map_err(anyhow_err)?
            .into_iter()
            .collect()
    };

    let matches = rows
        .into_iter()
        .map(|r| SeriesLookupMatch {
            canonical_title: titles.get(&r.series_id).cloned().unwrap_or_default(),
            series_id: r.series_id,
            provider: r.provider,
            external_id: r.external_id,
        })
        .collect();
    Ok(Json(SeriesLookupResponse { matches }))
}

/// Map a `(provider, id)` pair detected from a URL onto one that actually
/// exists in `series_external_ids`.
///
/// Only legacy MangaUpdates needs translating: `series.html?id=NNN` URLs
/// detect as the synthetic `mangaupdates-legacy` provider, which is never
/// stored — the resolver rewrites those to modern slugs via a cache. Querying
/// for it directly would always miss while looking like a supported URL, so
/// consult the same cache here. A cache miss yields no match rather than a
/// wrong one; resolving it for real needs a network redirect, which does not
/// belong in a lookup endpoint.
async fn resolve_detected_pair(
    state: &AppState,
    provider: &'static str,
    id: String,
) -> ApiResult<Option<(String, String)>> {
    if provider != td_resolution::foreign_id::MANGAUPDATES_LEGACY {
        return Ok(Some((provider.to_string(), id)));
    }
    let Ok(legacy) = id.parse::<i64>() else {
        return Ok(None);
    };
    let modern = td_db::repos::mangaupdates_id_repo::lookup(&state.db, legacy)
        .await
        .map_err(anyhow_err)?
        .flatten();
    Ok(modern.map(|m| ("mangaupdates".to_string(), m)))
}

/// Incremental release feed: series with coverage activity, ordered by
/// `(updatedAt, id)` after an opaque cursor. A consumer (e.g. a Codex release
/// plugin) polls this a few times a day, stores `nextCursor`, and only ever
/// receives series whose coverage changed since its last poll. Keyset, not
/// offset, so it's gap-free and dupe-free while series are re-stamped
/// concurrently; delivery is at-least-once, so consumers upsert by `seriesId`.
#[utoipa::path(
    get,
    path = "/api/v1/series/feed",
    params(SeriesFeedQuery),
    responses((status = 200, description = "A page of changed series", body = SeriesFeedResponse)),
    tag = "series",
)]
pub async fn feed(
    State(state): State<AppState>,
    Query(q): Query<SeriesFeedQuery>,
) -> ApiResult<Json<SeriesFeedResponse>> {
    Ok(Json(
        feed_page(&state, q.cursor.as_deref(), q.limit, &[]).await?,
    ))
}

/// Filtered variant of [`feed`]: same cursor walk, but narrowed to the series
/// whose provider ids the consumer sends in the body. Use this (over the `GET`)
/// when a consumer tracks a known subset — a Codex release plugin posting the
/// `provider:externalId` set it owns so it only receives changes it cares
/// about. It's a `POST` purely so the id list can be large; it mutates nothing
/// and is gated like the other reads.
#[utoipa::path(
    post,
    path = "/api/v1/series/feed",
    request_body = SeriesFeedRequest,
    responses((status = 200, description = "A page of changed series, filtered to the requested ids", body = SeriesFeedResponse)),
    tag = "series",
)]
pub async fn feed_query(
    State(state): State<AppState>,
    Json(req): Json<SeriesFeedRequest>,
) -> ApiResult<Json<SeriesFeedResponse>> {
    // "provider:externalId" → (provider, externalId). Tokens without a colon
    // are dropped leniently (a consumer always has both halves from the feed).
    let external_ids: Vec<(String, String)> = req
        .external_ids
        .iter()
        .filter_map(|t| {
            t.split_once(':')
                .map(|(p, e)| (p.to_string(), e.to_string()))
        })
        .collect();
    let limit = req.limit.unwrap_or(FEED_DEFAULT_LIMIT);
    Ok(Json(
        feed_page(&state, req.cursor.as_deref(), limit, &external_ids).await?,
    ))
}

/// Shared core for both feed handlers: decode the cursor, page the keyset
/// (optionally filtered to `external_ids`), and shape the response.
async fn feed_page(
    state: &AppState,
    cursor: Option<&str>,
    limit: u32,
    external_ids: &[(String, String)],
) -> ApiResult<SeriesFeedResponse> {
    let limit = limit.clamp(1, FEED_MAX_LIMIT) as u64;
    let (after_updated_at, after_id) = match cursor {
        Some(token) => decode_feed_cursor(token)
            .ok_or_else(|| ApiError::BadRequest(format!("invalid cursor: {token}")))?,
        None => (0, 0),
    };

    // Fetch one extra to detect whether more pages remain without a COUNT.
    let mut rows = series_repo::feed_after(
        &state.db,
        after_updated_at,
        after_id,
        external_ids,
        limit + 1,
    )
    .await
    .map_err(anyhow_err)?;
    let has_more = rows.len() as u64 > limit;
    rows.truncate(limit as usize);
    let next_cursor = rows.last().map(|s| encode_feed_cursor(s.updated_at, s.id));

    let ids: Vec<i32> = rows.iter().map(|s| s.id).collect();
    let externals = series_external_ids_repo::by_series_ids(&state.db, &ids)
        .await
        .map_err(anyhow_err)?;

    let items = rows
        .into_iter()
        .map(|s| {
            let external_ids = externals
                .get(&s.id)
                .into_iter()
                .flatten()
                .map(|m| ExternalIdDto {
                    provider: m.provider.clone(),
                    external_id: m.external_id.clone(),
                    fetched_at: m.fetched_at,
                })
                .collect();
            SeriesFeedItem {
                series_id: s.id,
                canonical_title: s.canonical_title,
                external_ids,
                volume_coverage: coverage_dto(s.volume_coverage_json.as_deref()),
                chapter_coverage: coverage_dto(s.chapter_coverage_json.as_deref()),
                highest_volume: s.highest_volume,
                highest_chapter: s.highest_chapter,
                updated_at: s.updated_at,
            }
        })
        .collect();

    Ok(SeriesFeedResponse {
        items,
        next_cursor,
        has_more,
    })
}

/// Parse a stored `*_coverage_json` column into the feed's span DTOs.
fn coverage_dto(json: Option<&str>) -> Vec<CoverageSpanDto> {
    td_source::spans_from_json(json)
        .into_iter()
        .map(|s| CoverageSpanDto {
            start: s.start,
            end: s.end,
        })
        .collect()
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
    /// `metadata.series_refresh.min_age_days`, or `0` for a `scope = "all"`
    /// drain (which ignores the floor).
    pub min_age_days: u32,
    /// `"settings"` for a single, settings-bounded tick (honors
    /// `batch_size` + `min_age_days`); `"all"` for a drain that re-fetches
    /// every eligible row in repeated batches, ignoring `min_age_days`.
    pub scope: String,
}

/// Query for `POST /api/v1/series/refresh-all`. `all=true` switches from a
/// single settings-bounded tick to a full drain.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct RefreshAllSeriesQuery {
    /// When `true`, ignore `min_age_days` and re-fetch *every* eligible
    /// (non-manual, provider-mapped) series in repeated `batch_size`
    /// chunks. Defaults to `false`: a single settings-bounded tick.
    #[serde(default)]
    pub all: bool,
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
            // Admin-gated route: the caller is always an admin, so surface the
            // wishlist flag.
            true,
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
        true,
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
        true,
    )))
}

/// Hydrate a series row into the admin `SeriesDetail` shape (provider mappings +
/// genres + tags + Codex overlay). For the admin-only write endpoints, which
/// always surface the operator-only fields, so `is_admin` is hard-coded `true`.
async fn load_admin_detail(state: &AppState, id: i32) -> ApiResult<SeriesDetail> {
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
    Ok(model_to_detail(
        row,
        mappings,
        genres_for_series,
        tags_for_series,
        codex,
        true,
    ))
}

/// Body for `PUT /api/v1/series/{id}/wishlist`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetWishlistedRequest {
    /// `true` to clip the series to the operator's wishlist, `false` to remove
    /// it. Re-clipping an already-wishlisted series refreshes its "clipped at".
    pub wishlisted: bool,
}

/// Clip or un-clip a series from the operator's wishlist — a curated "download
/// later" list. Works on **any** series (provider-backed or manual): the flag
/// is operator-owned and a metadata refresh never touches it. Independent of
/// Codex ownership; clipping a series the operator already owns is allowed and
/// import never auto-clears it (removal is manual).
#[utoipa::path(
    put,
    path = "/api/v1/series/{id}/wishlist",
    tag = "series",
    params(("id" = i32, Path, description = "Internal series id")),
    request_body = SetWishlistedRequest,
    responses(
        (status = 200, body = SeriesDetail),
        (status = 404, description = "No series with that id")
    ),
    security(("admin" = []))
)]
pub async fn set_wishlisted(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<SetWishlistedRequest>,
) -> ApiResult<Json<SeriesDetail>> {
    let now = Utc::now().timestamp();
    match series_repo::set_wishlisted(&state.db, id, req.wishlisted, now)
        .await
        .map_err(anyhow_err)?
    {
        series_repo::SetWishlistedOutcome::Updated(_) => {}
        series_repo::SetWishlistedOutcome::NotFound => {
            return Err(ApiError::NotFound(format!("series {id}")));
        }
    }
    // Re-hydrate via the read-path helper so the response matches GET.
    Ok(Json(load_admin_detail(&state, id).await?))
}

/// Body for `PUT /api/v1/series/bulk/wishlist`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BulkWishlistRequest {
    /// Series ids to clip or un-clip. Must be non-empty; ids with no series
    /// row are silently dropped from the `updated` count.
    pub ids: Vec<i32>,
    /// `true` clips every listed series (stamping a fresh "clipped at"),
    /// `false` removes them all. An explicit set, not a per-row toggle, so a
    /// mixed selection converges to one state instead of flipping each row.
    pub wishlisted: bool,
}

/// Response for `PUT /api/v1/series/bulk/wishlist`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BulkWishlistResponse {
    /// Number of series rows actually written.
    pub updated: u64,
}

/// Clip or un-clip a whole selection of series in one call — the selection-bar
/// counterpart of `PUT /series/{id}/wishlist`, with the same column semantics
/// (operator-owned flag, any provenance, refresh never touches it).
#[utoipa::path(
    put,
    path = "/api/v1/series/bulk/wishlist",
    tag = "series",
    request_body = BulkWishlistRequest,
    responses(
        (status = 200, body = BulkWishlistResponse),
        (status = 400, description = "Empty ids list")
    ),
    security(("admin" = []))
)]
pub async fn bulk_wishlist(
    State(state): State<AppState>,
    Json(req): Json<BulkWishlistRequest>,
) -> ApiResult<Json<BulkWishlistResponse>> {
    if req.ids.is_empty() {
        return Err(ApiError::BadRequest("ids must not be empty".into()));
    }
    let now = Utc::now().timestamp();
    let updated = series_repo::set_wishlisted_bulk(&state.db, &req.ids, req.wishlisted, now)
        .await
        .map_err(anyhow_err)?;
    Ok(Json(BulkWishlistResponse { updated }))
}

fn default_true() -> bool {
    true
}

/// Body for `POST /api/v1/series/from-provider`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateSeriesFromProviderRequest {
    /// Provider id the series is fetched from (e.g. `mangabaka`). Must be a
    /// registered provider.
    pub provider: String,
    /// The provider's own external id for the series.
    pub external_id: String,
    /// Whether to clip the created/looked-up series to the wishlist. Defaults
    /// to `true` — the add-from-search flow's whole reason to exist; pass
    /// `false` to just materialize the catalog row without wishlisting it.
    #[serde(default = "default_true")]
    pub wishlist: bool,
}

/// Add a series straight from a metadata provider, for series with no
/// discovered release yet. Reuses the resolver's `upsert_series_from_metadata`
/// (the same path the review link-by-provider flow uses), so the row is
/// provider-backed and carries its `series_external_ids` mapping — future
/// discovered releases auto-resolve to it. Idempotent on `(provider,
/// externalId)`: an existing mapping returns `200` with the existing row; a
/// fresh fetch returns `201`. When `wishlist` is set (the default) the series
/// is clipped to the wishlist.
#[utoipa::path(
    post,
    path = "/api/v1/series/from-provider",
    tag = "series",
    request_body = CreateSeriesFromProviderRequest,
    responses(
        (status = 201, body = SeriesDetail, description = "Series created from provider metadata"),
        (status = 200, body = SeriesDetail, description = "Series already existed for this (provider, externalId)"),
        (status = 400, description = "Empty fields or unregistered provider"),
        (status = 404, description = "Provider has no record for that external id")
    ),
    security(("admin" = []))
)]
pub async fn create_from_provider(
    State(state): State<AppState>,
    Json(req): Json<CreateSeriesFromProviderRequest>,
) -> ApiResult<(StatusCode, Json<SeriesDetail>)> {
    let provider = req.provider.trim();
    let external_id = req.external_id.trim();
    if provider.is_empty() || external_id.is_empty() {
        return Err(ApiError::BadRequest(
            "provider and externalId must not be empty".into(),
        ));
    }
    let now = Utc::now();

    // Idempotent: a series already mapped to this (provider, externalId) is
    // reused rather than re-fetched, so a double-add can't fork the catalog.
    let (series_id, created) =
        match series_external_ids_repo::find_series_id(&state.db, provider, external_id)
            .await
            .map_err(anyhow_err)?
        {
            Some(sid) => (sid, false),
            None => {
                let target = state.metadata.get(provider).ok_or_else(|| {
                    ApiError::BadRequest(format!("provider {provider:?} not registered"))
                })?;
                let metadata: SeriesMetadata = target
                    .get(external_id)
                    .await
                    .map_err(|e| ApiError::Internal(anyhow::anyhow!("provider.get failed: {e}")))?
                    .ok_or_else(|| {
                        ApiError::NotFound(format!(
                            "provider {provider:?} has no record for {external_id:?}"
                        ))
                    })?;
                // No release yet, so seed last_release_at from `now`. Conservative
                // default (`false`): if the foreign-id chain lands on a pre-existing
                // manual row, don't clobber it.
                let sid = persist::upsert_series_from_metadata(
                    &state.db,
                    provider,
                    &metadata,
                    now.timestamp(),
                    now,
                    false,
                )
                .await
                .map_err(ApiError::Internal)?
                .series_id;
                (sid, true)
            }
        };

    if req.wishlist {
        match series_repo::set_wishlisted(&state.db, series_id, true, now.timestamp())
            .await
            .map_err(anyhow_err)?
        {
            series_repo::SetWishlistedOutcome::Updated(_) => {}
            series_repo::SetWishlistedOutcome::NotFound => {
                return Err(ApiError::Internal(anyhow::anyhow!(
                    "series {series_id} vanished before wishlist stamp"
                )));
            }
        }
    }

    let detail = load_admin_detail(&state, series_id).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(detail)))
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
    match refresh_metadata_core(&state, id).await? {
        RefreshOutcome::Refreshed => {}
        RefreshOutcome::NotFound => {
            return Err(ApiError::NotFound(format!("series {id}")));
        }
        RefreshOutcome::NoActiveMapping { active_id } => {
            return Err(ApiError::Conflict(format!(
                "series {id} has no mapping for active provider {active_id:?}; link it manually first"
            )));
        }
        RefreshOutcome::ProviderMissingRecord {
            active_id,
            external_id,
        } => {
            return Err(ApiError::NotFound(format!(
                "active provider {active_id:?} has no record for {external_id}"
            )));
        }
    }
    // Re-hydrate via the read-path helper so the response matches GET (this
    // endpoint is admin-gated by the router, so the Codex overlay is included).
    Ok(Json(load_admin_detail(&state, id).await?))
}

/// Per-series outcome of a refresh attempt against the active provider.
/// Typed (rather than an error) so the single endpoint can map to its
/// specific 404/409 statuses while the bulk endpoint records per-id skips.
enum RefreshOutcome {
    Refreshed,
    NotFound,
    NoActiveMapping {
        active_id: String,
    },
    ProviderMissingRecord {
        active_id: String,
        external_id: String,
    },
}

/// Fetch one series' metadata from the active provider and re-persist it.
/// Only infrastructure faults (DB, provider I/O) surface as `Err`.
async fn refresh_metadata_core(state: &AppState, id: i32) -> ApiResult<RefreshOutcome> {
    if series::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(anyhow_err)?
        .is_none()
    {
        return Ok(RefreshOutcome::NotFound);
    }

    let active_id = state.metadata.active_id().to_string();
    let active = state.metadata.active().clone();
    let mappings = series_external_ids_repo::list_for_series(&state.db, id)
        .await
        .map_err(anyhow_err)?;
    let Some(active_mapping) = mappings.iter().find(|m| m.provider == active_id) else {
        return Ok(RefreshOutcome::NoActiveMapping { active_id });
    };

    let metadata: Option<SeriesMetadata> = active
        .get(&active_mapping.external_id)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("active.get failed: {e}")))?;
    let Some(metadata) = metadata else {
        return Ok(RefreshOutcome::ProviderMissingRecord {
            active_id,
            external_id: active_mapping.external_id.clone(),
        });
    };

    // Explicit operator action: the refresh buttons are the one path that
    // opts in to overwriting a manual row.
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
    Ok(RefreshOutcome::Refreshed)
}

/// Body for `POST /api/v1/series/bulk/refresh-metadata`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BulkRefreshMetadataRequest {
    /// Series ids to refresh. Must be non-empty.
    pub ids: Vec<i32>,
}

/// One series a bulk refresh could not rewrite, and why.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BulkRefreshSkipDto {
    pub id: i32,
    /// Human-readable skip reason ("series not found", "no mapping for the
    /// active provider", "provider has no record").
    pub reason: String,
}

/// Response for `POST /api/v1/series/bulk/refresh-metadata`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BulkRefreshMetadataResponse {
    /// Number of series rewritten from provider metadata.
    pub refreshed: u64,
    /// Series that could not be refreshed; the batch never aborts on these.
    pub skipped: Vec<BulkRefreshSkipDto>,
}

/// Refresh a whole selection of series from the active provider in one call.
///
/// Runs synchronously: the active provider today resolves `get()` against its
/// local offline dump, so a page-sized batch is fast and network-free. If a
/// future active provider fetches remotely per `get()`, this must move to a
/// dispatched background job with progress reporting instead.
///
/// Per-id problems (unknown id, no active-provider mapping, provider record
/// missing) are reported in `skipped` — the batch never aborts on them.
#[utoipa::path(
    post,
    path = "/api/v1/series/bulk/refresh-metadata",
    tag = "series",
    request_body = BulkRefreshMetadataRequest,
    responses(
        (status = 200, body = BulkRefreshMetadataResponse),
        (status = 400, description = "Empty ids list")
    ),
    security(("admin" = []))
)]
pub async fn bulk_refresh_metadata(
    State(state): State<AppState>,
    Json(req): Json<BulkRefreshMetadataRequest>,
) -> ApiResult<Json<BulkRefreshMetadataResponse>> {
    if req.ids.is_empty() {
        return Err(ApiError::BadRequest("ids must not be empty".into()));
    }
    let mut refreshed = 0u64;
    let mut skipped = Vec::new();
    for id in &req.ids {
        match refresh_metadata_core(&state, *id).await? {
            RefreshOutcome::Refreshed => refreshed += 1,
            RefreshOutcome::NotFound => skipped.push(BulkRefreshSkipDto {
                id: *id,
                reason: "series not found".into(),
            }),
            RefreshOutcome::NoActiveMapping { active_id } => skipped.push(BulkRefreshSkipDto {
                id: *id,
                reason: format!("no mapping for active provider {active_id:?}"),
            }),
            RefreshOutcome::ProviderMissingRecord {
                active_id,
                external_id,
            } => skipped.push(BulkRefreshSkipDto {
                id: *id,
                reason: format!("active provider {active_id:?} has no record for {external_id}"),
            }),
        }
    }
    Ok(Json(BulkRefreshMetadataResponse { refreshed, skipped }))
}

/// Trigger a bulk refresh of stale series rows against the active
/// metadata provider.
///
/// Default (`all=false`): one settings-bounded tick. It reads `batch_size`
/// and `min_age_days` from `metadata.series_refresh`; the same selection
/// query backs the cron, so a manual click and a cron tick are
/// behaviourally identical.
///
/// `all=true`: a drain that re-fetches *every* eligible (non-manual,
/// provider-mapped) row in repeated `batch_size` chunks, ignoring the
/// `min_age_days` floor, until none remain.
///
/// Both modes share the per-provider mutex (so they can't race the cron or
/// each other) and return immediately with `triggered: true` once the work
/// is spawned, or `triggered: false, skipped: true` when a refresh is
/// already in flight for the active provider.
#[utoipa::path(
    post,
    path = "/api/v1/series/refresh-all",
    tag = "series",
    operation_id = "refresh_all_series",
    params(RefreshAllSeriesQuery),
    responses(
        (status = 202, body = RefreshAllSeriesResponse),
        (status = 503, description = "Active provider is not registered")
    ),
    security(("admin" = []))
)]
pub async fn refresh_all(
    State(state): State<AppState>,
    Query(query): Query<RefreshAllSeriesQuery>,
) -> ApiResult<Json<RefreshAllSeriesResponse>> {
    let active_id = state.metadata.active_id().to_string();
    let provider = state.metadata.active().clone();
    let batch_size = state.metadata_config.series_refresh.batch_size;
    let drain = query.all;
    // A drain ignores the min-age floor; report 0 so the response is honest.
    let min_age_days = if drain {
        0
    } else {
        state.metadata_config.series_refresh.min_age_days
    };
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
            if drain {
                refresh_series_metadata::run_drain(
                    provider,
                    db,
                    batch_size,
                    events,
                    run_metrics_repo::trigger::MANUAL,
                )
                .await;
            } else {
                refresh_series_metadata::run_tick(
                    provider,
                    db,
                    batch_size,
                    min_age_seconds,
                    events,
                    run_metrics_repo::trigger::MANUAL,
                )
                .await;
            }
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
        scope: if drain { "all" } else { "settings" }.into(),
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
    let now = Utc::now().timestamp();
    let summary = releases_repo::recompute_all_spans(&state.db, now)
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
    is_admin: bool,
) -> SeriesListItem {
    // Wishlist is operator curation — gate it behind admin like `owned`, so the
    // public read tier never learns what's on the list.
    let wishlisted_at = if is_admin { m.wishlisted_at } else { None };
    SeriesListItem {
        id: m.id,
        canonical_title: m.canonical_title,
        cover_url: m.cover_url,
        kind: m.kind,
        status: m.status,
        year: m.year,
        published_start_date: m.published_start_date,
        published_end_date: m.published_end_date,
        description: m.description,
        genres,
        tags,
        metadata_source: m.metadata_source,
        last_release_at: m.last_release_at,
        first_seen_at: m.first_seen_at,
        last_discovered_at: m.last_discovered_at,
        release_count,
        total_volumes: m.total_volumes,
        total_chapters: m.total_chapters,
        highest_volume: m.highest_volume,
        highest_chapter: m.highest_chapter,
        rating: m.rating,
        // `owned` now reflects Codex ownership, surfaced only to admins (the
        // legacy `series.owned` column was never populated).
        owned: codex.is_some(),
        wishlisted: wishlisted_at.is_some(),
        wishlisted_at,
        codex,
    }
}

fn model_to_detail(
    m: series::Model,
    mappings: Vec<series_external_ids::Model>,
    join_genres: Vec<String>,
    join_tags: Vec<String>,
    codex: Option<CodexInfo>,
    is_admin: bool,
) -> SeriesDetail {
    // Admin-gated, same rationale as `model_to_list_item`.
    let wishlisted_at = if is_admin { m.wishlisted_at } else { None };
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
        published_start_date: m.published_start_date,
        published_end_date: m.published_end_date,
        description: m.description,
        genres: join_genres,
        tags: join_tags,
        metadata_source: m.metadata_source,
        metadata_fetched_at: m.metadata_fetched_at,
        first_seen_at: m.first_seen_at,
        last_release_at: m.last_release_at,
        last_discovered_at: m.last_discovered_at,
        highest_volume: m.highest_volume,
        highest_chapter: m.highest_chapter,
        total_volumes: m.total_volumes,
        total_chapters: m.total_chapters,
        rating: m.rating,
        owned: codex.is_some(),
        wishlisted: wishlisted_at.is_some(),
        wishlisted_at,
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
            build_fts_match_expression("solo leveling", true),
            "\"solo\" \"leveling\"*"
        );
    }

    #[test]
    fn fts_match_treats_punctuation_as_token_separator() {
        // Mixed punctuation, including a leading bracket that would
        // otherwise be a syntax error in a raw FTS5 expression.
        assert_eq!(
            build_fts_match_expression("[scanlator] Solo-Leveling!", true),
            "\"scanlator\" \"Solo\" \"Leveling\"*"
        );
    }

    #[test]
    fn fts_match_returns_empty_when_no_alphanumeric_tokens() {
        // Empty regardless of mode — no tokens to scope.
        assert_eq!(build_fts_match_expression("!!! ???", true), "");
        assert_eq!(build_fts_match_expression("", true), "");
        assert_eq!(build_fts_match_expression("!!! ???", false), "");
        assert_eq!(build_fts_match_expression("", false), "");
    }

    #[test]
    fn fts_match_handles_single_token_with_prefix() {
        assert_eq!(build_fts_match_expression("naruto", true), "\"naruto\"*");
    }

    #[test]
    fn fts_match_scopes_to_titles_when_description_excluded() {
        // Default mode pins matching to the title columns; the parentheses
        // are required so the column filter spans every token, not just the
        // first. This must reproduce the pre-description-column behavior.
        assert_eq!(
            build_fts_match_expression("solo leveling", false),
            "{title alternate_titles} : (\"solo\" \"leveling\"*)"
        );
        assert_eq!(
            build_fts_match_expression("naruto", false),
            "{title alternate_titles} : (\"naruto\"*)"
        );
    }
}
