//! Release endpoints: list / unresolved feed / link / reject / retry.

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::Utc;
use sea_orm::sea_query::Query as SeaQuery;
use sea_orm::{
    ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Select,
};
use serde::{Deserialize, Serialize};
use td_db::entities::{release_formats, releases};
use td_db::repos::{releases_repo, review_repo, series_external_ids_repo};
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
    /// Raw description blob (markdown for Nyaa posts that ran detail fetch;
    /// the RSS anchor stub otherwise). Omitted when absent. Surfaced so the
    /// review and kept views can render it inline without opening the post.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description_html: Option<String>,
    /// External-provider links scraped from the description. Omitted when
    /// nothing was found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extracted_links: Option<ExtractedLinksDto>,
    /// Provider links found in the post's *comments* — untrusted, so they are
    /// never auto-resolved. The review UI offers them as one-click lookups the
    /// operator can confirm. Omitted when absent. Distinct from
    /// `extracted_links` (the uploader's links).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_suggested_links: Option<ExtractedLinksDto>,
    /// URL from the post's "Information" field, verbatim. Surfaced even when
    /// it is not a provider link we resolve against (a publisher page, a
    /// Discord invite, …) so the review and kept views show the uploader's
    /// cited source. Omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub information_url: Option<String>,
    /// Epoch seconds the release was pushed to the torrent client, or omitted
    /// if it was never sent. Drives the "Sent" badge on the review/kept cards.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_to_client_at: Option<i64>,
    /// Label the release was sent to the torrent client with. Omitted when
    /// absent (never sent, or sent without a label).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_to_client_label: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReleasePage {
    pub items: Vec<ReleaseDto>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCandidateDto {
    pub series_id: i32,
    pub series_title: String,
    pub series_cover_url: Option<String>,
    /// Series format (manga / novel / manhwa / …) from provider metadata.
    /// Surfaced so the operator can sanity-check a candidate's format
    /// against the release before linking. `None` when the provider did
    /// not classify the series.
    pub kind: Option<String>,
    /// Published volume/chapter counts from provider metadata. Let the
    /// operator compare the release's contents (see the torrent file list)
    /// against the candidate series' length. `None` when the provider did
    /// not expose them or the series predates this field.
    pub total_volumes: Option<i32>,
    pub total_chapters: Option<i32>,
    pub score: f64,
    pub reason: Option<String>,
    /// Provider + external_id pair for building a link to the provider's
    /// page. Prefers the active provider's mapping; falls back to any
    /// other mapping so the operator always has a way to inspect the
    /// candidate. None when the series has no external IDs persisted.
    pub provider: Option<String>,
    pub external_id: Option<String>,
    /// Alternate / native titles persisted on the series row. Surfaced
    /// in the review UI so romaji / Japanese / publisher variants are
    /// visible without opening the provider page.
    pub alternate_titles: Vec<String>,
}

/// External provider links scraped from a release's description. Mirrors
/// `td_source::ExternalLinks`; redefined here so we don't pull utoipa into
/// the domain crate.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedLinksDto {
    pub mangaupdates: Option<String>,
    pub anilist: Option<String>,
    pub mal: Option<String>,
    pub mangadex: Option<String>,
    #[serde(default)]
    pub mangabaka: Option<String>,
}

impl ExtractedLinksDto {
    fn is_empty(&self) -> bool {
        self.mangaupdates.is_none()
            && self.anilist.is_none()
            && self.mal.is_none()
            && self.mangadex.is_none()
            && self.mangabaka.is_none()
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedRelease {
    #[serde(flatten)]
    pub release: ReleaseDto,
    pub candidates: Vec<ReviewCandidateDto>,
    /// Search queries the title cleaner produced for this release
    /// (longest-first). Empty when the release predates the cleaner or
    /// failed to persist; the next resolve cycle backfills.
    pub search_queries: Vec<String>,
    /// Stable rule names that fired during cleanup (e.g. `strip_parens`,
    /// `split_alternates`). Rendered as badge chips on the review card.
    pub cleanup_rules_applied: Vec<String>,
    /// Convenience pointer to `candidates[0]`, when present. Lets the
    /// card render without defensive-checking the array on every render.
    pub top_candidate: Option<ReviewCandidateDto>,
    // `description_html` and `extracted_links` are carried by the flattened
    // `ReleaseDto` above (shared with the kept/feed views).
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedPage {
    pub items: Vec<UnresolvedRelease>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetryAllResponse {
    /// `true` when a batch was spawned by this request.
    pub triggered: bool,
    /// `true` when a prior retry-all batch is still in flight; the request is a no-op.
    pub skipped: bool,
}

/// Per-batch ceiling for `POST /releases/retry-all`. Matches the CLI's
/// default and is plenty for personal-scale review queues.
const RETRY_ALL_BATCH_LIMIT: u64 = 1000;

/// Shared body for the bulk review actions. The target set is either an
/// explicit `ids` list (when non-empty) or every queue release matching the
/// filter fields. An all-empty body targets the entire queue. The filters
/// mirror [`ReviewQueueQuery`] so "select all matching" acts on exactly what
/// the list endpoint shows; explicit `ids` are still intersected with the
/// queue statuses so a decided release can't be re-acted on.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct BulkReviewRequest {
    pub ids: Vec<String>,
    pub q: Option<String>,
    pub source_name: Option<String>,
    pub format: Option<String>,
    pub status: Option<String>,
    /// Release-group filter mirroring [`ReviewQueueQuery::search_query`], so
    /// "reject/retry the whole group" acts on exactly the releases the chip
    /// shows.
    pub search_query: Option<String>,
    /// Grouping breadth mirroring [`ReviewQueueQuery::breadth`] (clamped to
    /// {1,2,3}, default 1).
    pub breadth: Option<u8>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BulkRejectResponse {
    /// Number of releases moved to `rejected`.
    pub rejected: u64,
}

/// Body for [`bulk_link`]: link every release in `ids` to a single series.
/// The target series is named exactly as in [`LinkRequest`] — either an
/// existing `seriesId`, or a `provider` + `externalId` pair the active
/// provider can resolve (materializing the series row on first link). The
/// `ids` are intersected with the queue statuses server-side so a decided
/// release can't be relinked, and an empty `ids` list is rejected: linking a
/// whole filtered set to one series is never what the operator means.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct BulkLinkRequest {
    pub ids: Vec<String>,
    pub series_id: Option<i32>,
    pub provider: Option<String>,
    pub external_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BulkLinkResponse {
    /// Number of releases linked to the target series.
    pub linked: u64,
    /// The series the releases were linked to.
    pub series_id: i32,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BulkRetryResponse {
    /// `true` when a batch was spawned by this request.
    pub triggered: bool,
    /// `true` when a prior retry batch is still in flight; the request is a no-op.
    pub skipped: bool,
    /// Number of releases the filters/ids matched (the batch size, capped at
    /// the per-batch limit).
    pub matched: u64,
}

/// A cluster of review-queue releases sharing one cleaned search query. The
/// `query` is exactly the value to hand back as `searchQuery` (at the same
/// `breadth`) to filter the list down to this group's members.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseGroupDto {
    /// The shared cleaned search query, and the `searchQuery` filter value
    /// that reproduces this group.
    pub query: String,
    /// Number of distinct releases in the group (always > 1).
    pub count: i64,
    /// Optional display-only hint about the group's most common top-candidate
    /// series. Populated in a later phase; currently always absent. Never a
    /// grouping key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_candidate: Option<ReleaseGroupCandidateDto>,
}

/// Display-only top-candidate hint attached to a [`ReleaseGroupDto`].
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseGroupCandidateDto {
    pub series_id: i32,
    pub title: String,
    pub cover_url: Option<String>,
}

/// Response for the grouped review-queue endpoint.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseGroupsResponse {
    /// The breadth the groups were computed at (clamped to {1,2,3}). Echoed so
    /// the client can pass it back as the list `breadth` unchanged.
    pub breadth: u8,
    /// Clusters with more than one member, ordered by descending count then
    /// query ascending.
    pub groups: Vec<ReleaseGroupDto>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(default, rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct ReleaseListQuery {
    pub page: u32,
    pub page_size: u32,
    /// Filter by resolution status (`resolved`, `unresolved`, `ambiguous`,
    /// `review_pending`, `rejected`, `standalone`).
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

/// The three statuses that make up the review queue. A release leaves the
/// queue once it's `resolved` or `rejected`; the filtered list and the bulk
/// actions are always scoped to this set so neither can touch a decided row.
const QUEUE_STATUSES: [&str; 3] = ["unresolved", "ambiguous", "review_pending"];

#[derive(Debug, Deserialize, IntoParams)]
#[serde(default, rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct ReviewQueueQuery {
    /// 1-indexed page number.
    pub page: u32,
    /// Items per page (capped server-side at 200).
    pub page_size: u32,
    /// Free-text title substring match (case-insensitive). Whitespace-only
    /// is treated as absent.
    pub q: Option<String>,
    /// Restrict to a single source instance (`releases.source_name`).
    pub source_name: Option<String>,
    /// Restrict to releases carrying this file format (e.g. `cbz`, `epub`).
    pub format: Option<String>,
    /// Narrow to one queue status. Ignored unless it's one of
    /// `unresolved` / `ambiguous` / `review_pending`.
    pub status: Option<String>,
    /// Group filter: select only releases whose cleaned `search_queries`
    /// contains this exact value (within the [`breadth`] index bound). Kept
    /// distinct from the title-substring [`q`] filter — this is the handoff
    /// from a clicked release group, not free-text search. Whitespace-only is
    /// treated as absent.
    pub search_query: Option<String>,
    /// How many leading `search_queries` variants the [`search_query`] /
    /// grouping match considers: `1` = primary `[0]` only (default), `2` =
    /// `[0..2)`, `3` = all elements. Out-of-range/absent clamps to `1`.
    pub breadth: Option<u8>,
    /// Result ordering. One of `observed_desc` (default), `observed_asc`,
    /// `posted_desc`, `posted_asc`, `title_asc`, `title_desc`. Title sorts
    /// are case-insensitive. An unknown value falls back to `observed_desc`.
    pub sort: Option<String>,
}

impl Default for ReviewQueueQuery {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: 50,
            q: None,
            source_name: None,
            format: None,
            status: None,
            search_query: None,
            breadth: None,
            sort: None,
        }
    }
}

/// Clamp the `breadth` knob to {1, 2, 3}, defaulting to `1` (tightest) for
/// `None` or any out-of-range value. Shared by the `search_query` list filter
/// and the grouped endpoint so a clicked group reproduces its own membership
/// at the same looseness.
fn clamp_breadth(breadth: Option<u8>) -> u8 {
    match breadth {
        Some(2) => 2,
        Some(3) => 3,
        _ => 1,
    }
}

impl ReviewQueueQuery {
    fn pagination(&self) -> Pagination {
        Pagination {
            page: self.page,
            page_size: self.page_size,
        }
    }
}

/// Build the base `Select` for the review queue from the optional filters.
/// Shared by the list endpoint and the bulk actions so "what you see" and
/// "what you act on" can never diverge.
///
/// - Always scoped to [`QUEUE_STATUSES`]; a `status` outside that set is
///   ignored (falls back to the full three-status set) so the queue never
///   surfaces a `resolved`/`rejected` row.
/// - `q` is a `title LIKE '%q%'` substring match (SQLite ASCII LIKE is
///   case-insensitive). Raw `%`/`_` act as wildcards — acceptable for a
///   single-user title search.
/// - `format` filters via the `release_formats` join table by subquery, since
///   a release can carry several formats.
/// - `search_query` (the release-group filter) matches a cleaned query against
///   the flattened `search_queries` JSON array via a correlated `json_each`
///   `EXISTS`, bounded by `breadth`. This is the same predicate the grouped
///   endpoint groups on (see [`search_query_exists`]) so a clicked group
///   reproduces its own membership exactly.
fn review_queue_select(
    q: Option<&str>,
    source_name: Option<&str>,
    format: Option<&str>,
    status: Option<&str>,
    search_query: Option<&str>,
    breadth: u8,
) -> Select<releases::Entity> {
    fn trimmed(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|s| !s.is_empty())
    }

    let mut select = releases::Entity::find();
    match trimmed(status) {
        Some(s) if QUEUE_STATUSES.contains(&s) => {
            select = select.filter(releases::Column::ResolutionStatus.eq(s));
        }
        _ => {
            select = select.filter(releases::Column::ResolutionStatus.is_in(QUEUE_STATUSES));
        }
    }
    if let Some(q) = trimmed(q) {
        select = select.filter(releases::Column::Title.contains(q));
    }
    if let Some(name) = trimmed(source_name) {
        select = select.filter(releases::Column::SourceName.eq(name));
    }
    if let Some(fmt) = trimmed(format) {
        let sub = SeaQuery::select()
            .column(release_formats::Column::ReleaseId)
            .from(release_formats::Entity)
            .and_where(release_formats::Column::Format.eq(fmt))
            .to_owned();
        select = select.filter(releases::Column::Id.in_subquery(sub));
    }
    if let Some(sq) = trimmed(search_query) {
        select = select.filter(search_query_exists(sq, breadth));
    }
    select
}

/// The `breadth` index bound applied to the flattened `search_queries` array,
/// as a raw SQL fragment (with the leading `AND`). `breadth = 3` means "all
/// elements," so it drops the bound entirely. The two callers (the list filter
/// in [`review_queue_select`] and the grouped aggregation query) must share
/// this so group membership and group filtering can't diverge.
fn breadth_key_bound(breadth: u8) -> &'static str {
    match breadth {
        2 => " AND je.key < 2",
        3 => "",
        _ => " AND je.key = 0",
    }
}

/// Correlated `EXISTS` predicate matching releases whose `search_queries`
/// array (within the `breadth` index bound) contains `value` exactly. The
/// `je` alias must match the bound produced by [`breadth_key_bound`].
fn search_query_exists(value: &str, breadth: u8) -> sea_orm::sea_query::SimpleExpr {
    use sea_orm::sea_query::Expr;
    let bound = breadth_key_bound(breadth);
    let sql = format!(
        "EXISTS (SELECT 1 FROM json_each(releases.search_queries) je \
         WHERE je.value = ?{bound})"
    );
    Expr::cust_with_values(&sql, [value])
}

/// Apply the review-queue ordering selected by the `sort` query param.
/// Title sorts compare case-insensitively (`lower(title)`) and break ties on
/// `id` so pagination is stable across pages. Every other value (and the
/// default / unknown case) orders by `observed_at` descending — newest first,
/// matching the rest of the release views.
fn apply_review_sort(
    select: Select<releases::Entity>,
    sort: Option<&str>,
) -> Select<releases::Entity> {
    use sea_orm::Order;
    use sea_orm::sea_query::{Expr, Func};

    let lower_title = || Expr::expr(Func::lower(Expr::col(releases::Column::Title)));
    match sort.map(str::trim) {
        Some("observed_asc") => select.order_by_asc(releases::Column::ObservedAt),
        Some("posted_desc") => select.order_by_desc(releases::Column::PostedAt),
        Some("posted_asc") => select.order_by_asc(releases::Column::PostedAt),
        Some("title_asc") => select
            .order_by(lower_title(), Order::Asc)
            .order_by_asc(releases::Column::Id),
        Some("title_desc") => select
            .order_by(lower_title(), Order::Desc)
            .order_by_asc(releases::Column::Id),
        _ => select.order_by_desc(releases::Column::ObservedAt),
    }
}

/// Resolve a [`BulkReviewRequest`] into the concrete release ids it targets,
/// always scoped to the queue statuses via [`review_queue_select`]. When
/// `ids` are given they further restrict the set (so a stale/decided id is
/// dropped); otherwise the filters select the whole matching set. `limit`
/// caps the materialized set for the (expensive, per-item) retry path; reject
/// passes `None` since it's a single set-based update.
async fn review_queue_target_ids(
    db: &sea_orm::DatabaseConnection,
    req: &BulkReviewRequest,
    limit: Option<u64>,
) -> ApiResult<Vec<String>> {
    let mut select = review_queue_select(
        req.q.as_deref(),
        req.source_name.as_deref(),
        req.format.as_deref(),
        req.status.as_deref(),
        req.search_query.as_deref(),
        clamp_breadth(req.breadth),
    );
    if !req.ids.is_empty() {
        select = select.filter(releases::Column::Id.is_in(req.ids.iter().cloned()));
    }
    select = select.order_by_desc(releases::Column::ObservedAt);
    if let Some(limit) = limit {
        select = select.limit(limit);
    }
    let ids = select
        .select_only()
        .column(releases::Column::Id)
        .into_tuple::<String>()
        .all(db)
        .await
        .map_err(anyhow_err)?;
    Ok(ids)
}

/// List releases ordered by `observed_at` descending. Filters compose.
#[utoipa::path(
    get,
    path = "/api/v1/releases",
    tag = "releases",
    operation_id = "list_releases",
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
        .order_by_desc(releases::Column::PostedAt)
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
    params(ReviewQueueQuery),
    responses((status = 200, body = UnresolvedPage))
)]
pub async fn list_unresolved(
    State(state): State<AppState>,
    Query(query): Query<ReviewQueueQuery>,
) -> ApiResult<Json<UnresolvedPage>> {
    let p = query.pagination();
    let select = review_queue_select(
        query.q.as_deref(),
        query.source_name.as_deref(),
        query.format.as_deref(),
        query.status.as_deref(),
        query.search_query.as_deref(),
        clamp_breadth(query.breadth),
    );
    let total = select.clone().count(&state.db).await.map_err(anyhow_err)?;
    let rows = apply_review_sort(select, query.sort.as_deref())
        .offset(p.offset())
        .limit(p.limit())
        .all(&state.db)
        .await
        .map_err(anyhow_err)?;

    let active_provider = state.metadata.active_id().to_string();
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
            let alternate_titles = series
                .as_ref()
                .and_then(|s| s.alternate_titles_json.as_deref())
                .and_then(|j| serde_json::from_str::<Vec<String>>(j).ok())
                .unwrap_or_default();
            // Prefer the active provider's mapping (almost always present
            // since the candidate came from that provider's resolver);
            // fall back to any other mapping so we never leave the
            // operator without a way to inspect the series. The UI builds
            // the actual URL from (provider, external_id).
            let mappings = series_external_ids_repo::list_for_series(&state.db, c.series_id)
                .await
                .map_err(anyhow_err)?;
            let picked = mappings
                .iter()
                .find(|m| m.provider == active_provider)
                .or_else(|| mappings.first());
            let provider = picked.map(|m| m.provider.clone());
            let external_id = picked.map(|m| m.external_id.clone());
            candidates.push(ReviewCandidateDto {
                series_id: c.series_id,
                series_title: series
                    .as_ref()
                    .map(|s| s.canonical_title.clone())
                    .unwrap_or_default(),
                total_volumes: series.as_ref().and_then(|s| s.total_volumes),
                total_chapters: series.as_ref().and_then(|s| s.total_chapters),
                kind: series.as_ref().and_then(|s| s.kind.clone()),
                series_cover_url: series.and_then(|s| s.cover_url),
                score: c.score,
                reason: c.reason,
                provider,
                external_id,
                alternate_titles,
            });
        }
        let search_queries: Vec<String> = row
            .search_queries
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default();
        let cleanup_rules_applied: Vec<String> = row
            .cleanup_rules_applied
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default();
        let top_candidate = candidates.first().cloned();
        items.push(UnresolvedRelease {
            release: model_to_release(row, formats),
            candidates,
            search_queries,
            cleanup_rules_applied,
            top_candidate,
        });
    }

    Ok(Json(UnresolvedPage {
        items,
        page: p.page(),
        page_size: p.page_size(),
        total,
    }))
}

/// Row shape for the grouped aggregation query.
#[derive(Debug, sea_orm::FromQueryResult)]
struct ReleaseGroupRow {
    query: String,
    n: i64,
}

/// Run the flatten/group/having aggregation over the review queue, returning
/// query-keyed clusters with more than one member.
///
/// This is the **one** place that reimplements the list's `q` / `source_name`
/// / `format` / `status` predicates in raw SQL — `json_each` is a
/// table-valued function sea-orm's entity builder doesn't model cleanly, so
/// the grouped query can't reuse [`review_queue_select`]. The WHERE fragments
/// below mirror that function verbatim, and the breadth index bound is shared
/// via [`breadth_key_bound`], so group membership here and group *filtering*
/// there can't diverge. The parity test is the gate that keeps them honest.
async fn release_groups(
    db: &sea_orm::DatabaseConnection,
    q: Option<&str>,
    source_name: Option<&str>,
    format: Option<&str>,
    status: Option<&str>,
    breadth: u8,
) -> ApiResult<Vec<ReleaseGroupDto>> {
    use sea_orm::{ConnectionTrait, FromQueryResult, Statement};

    fn trimmed(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|s| !s.is_empty())
    }

    let mut clauses: Vec<String> = Vec::new();
    let mut values: Vec<sea_orm::Value> = Vec::new();

    // Status: a single in-queue status pins to it; anything else (including
    // resolved/rejected/bogus) falls back to the full three-status set, so the
    // grouped view can never surface a decided row. Mirrors review_queue_select.
    match trimmed(status) {
        Some(s) if QUEUE_STATUSES.contains(&s) => {
            clauses.push("r.resolution_status = ?".to_string());
            values.push(s.into());
        }
        _ => {
            let placeholders = vec!["?"; QUEUE_STATUSES.len()].join(",");
            clauses.push(format!("r.resolution_status IN ({placeholders})"));
            for st in QUEUE_STATUSES {
                values.push(st.into());
            }
        }
    }
    if let Some(q) = trimmed(q) {
        // `LIKE '%q%'` with raw `%`/`_` acting as wildcards — matches
        // `Column::contains` in review_queue_select exactly.
        clauses.push("r.title LIKE ?".to_string());
        values.push(format!("%{q}%").into());
    }
    if let Some(name) = trimmed(source_name) {
        clauses.push("r.source_name = ?".to_string());
        values.push(name.into());
    }
    if let Some(fmt) = trimmed(format) {
        clauses
            .push("r.id IN (SELECT release_id FROM release_formats WHERE format = ?)".to_string());
        values.push(fmt.into());
    }

    let backend = db.get_database_backend();
    let where_sql = clauses.join(" AND ");
    let key_bound = breadth_key_bound(breadth);
    let sql = format!(
        "SELECT je.value AS query, COUNT(DISTINCT r.id) AS n \
         FROM releases r, json_each(r.search_queries) je \
         WHERE {where_sql}{key_bound} \
         GROUP BY je.value HAVING n > 1 \
         ORDER BY n DESC, je.value ASC"
    );
    // Same value bindings drive both statements; clone since each Statement
    // takes ownership.
    let stmt = Statement::from_sql_and_values(backend, &sql, values.clone());
    let rows = ReleaseGroupRow::find_by_statement(stmt)
        .all(db)
        .await
        .map_err(anyhow_err)?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    // Display-only hint: the dominant candidate series per group. Counts how
    // many distinct releases in each group carry a given candidate series and
    // picks the most common one (ties broken by higher total score, then lower
    // id). Never a grouping key — purely a "likely <title>" label.
    let dominant = dominant_candidate_per_group(db, backend, &where_sql, key_bound, values).await?;

    let mut groups = Vec::with_capacity(rows.len());
    for row in rows {
        let top_candidate = match dominant.get(&row.query) {
            Some(&series_id) => td_db::repos::series_repo::find_by_id(db, series_id)
                .await
                .map_err(anyhow_err)?
                .map(|s| ReleaseGroupCandidateDto {
                    series_id,
                    title: s.canonical_title,
                    cover_url: s.cover_url,
                }),
            None => None,
        };
        groups.push(ReleaseGroupDto {
            query: row.query,
            count: row.n,
            top_candidate,
        });
    }
    Ok(groups)
}

/// Row shape for the per-group dominant-candidate aggregation.
#[derive(Debug, sea_orm::FromQueryResult)]
struct GroupCandidateRow {
    query: String,
    series_id: i32,
    n: i64,
    score_sum: f64,
}

/// For each group (keyed by the flattened query value), find the candidate
/// series carried by the most distinct releases in that group. Returns a
/// `query -> series_id` map; queries whose releases have no candidates are
/// absent. The WHERE/breadth fragments mirror the main grouping query exactly
/// (same `values` bindings) so the hint is computed over the same membership.
async fn dominant_candidate_per_group(
    db: &sea_orm::DatabaseConnection,
    backend: sea_orm::DatabaseBackend,
    where_sql: &str,
    key_bound: &str,
    values: Vec<sea_orm::Value>,
) -> ApiResult<std::collections::HashMap<String, i32>> {
    use sea_orm::{FromQueryResult, Statement};

    let sql = format!(
        "SELECT je.value AS query, rc.series_id AS series_id, \
                COUNT(DISTINCT r.id) AS n, SUM(rc.score) AS score_sum \
         FROM releases r, json_each(r.search_queries) je \
         JOIN review_candidates rc ON rc.release_id = r.id \
         WHERE {where_sql}{key_bound} \
         GROUP BY je.value, rc.series_id"
    );
    let stmt = Statement::from_sql_and_values(backend, &sql, values);
    let rows = GroupCandidateRow::find_by_statement(stmt)
        .all(db)
        .await
        .map_err(anyhow_err)?;

    // Reduce to the single best candidate per query.
    let mut best: std::collections::HashMap<String, (i32, i64, f64)> =
        std::collections::HashMap::new();
    for c in rows {
        let replace = match best.get(&c.query) {
            None => true,
            Some(&(sid, n, score)) => {
                c.n > n
                    || (c.n == n && c.score_sum > score)
                    || (c.n == n && c.score_sum == score && c.series_id < sid)
            }
        };
        if replace {
            best.insert(c.query, (c.series_id, c.n, c.score_sum));
        }
    }
    Ok(best
        .into_iter()
        .map(|(query, (series_id, _, _))| (query, series_id))
        .collect())
}

/// Grouped review queue: clusters of releases sharing a cleaned search query.
///
/// Honors the same `q` / `sourceName` / `format` / `status` filters as the
/// list endpoint, plus `breadth` (1 = primary query only, default; 2 = first
/// two; 3 = all). Only clusters with more than one member are returned,
/// ordered by descending count. Clicking a group re-filters the list via
/// `searchQuery` (at the echoed `breadth`) to reproduce exactly these members.
/// Pagination/sort params are ignored.
#[utoipa::path(
    get,
    path = "/api/v1/releases/unresolved/groups",
    tag = "releases",
    params(ReviewQueueQuery),
    responses((status = 200, body = ReleaseGroupsResponse))
)]
pub async fn list_groups(
    State(state): State<AppState>,
    Query(query): Query<ReviewQueueQuery>,
) -> ApiResult<Json<ReleaseGroupsResponse>> {
    let breadth = clamp_breadth(query.breadth);
    let groups = release_groups(
        &state.db,
        query.q.as_deref(),
        query.source_name.as_deref(),
        query.format.as_deref(),
        query.status.as_deref(),
        breadth,
    )
    .await?;
    Ok(Json(ReleaseGroupsResponse { breadth, groups }))
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
    let series_id = resolve_link_target(&state, release.posted_at, req, now).await?;

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

/// Mark a release as a worthwhile standalone item that is not (and will
/// never be) a tracked series: a guidebook, an artbook, a one-shot. Drops
/// candidates and pins the status to `standalone` so the resolver leaves it
/// alone; unlike `rejected`, these stay browsable in the "Kept" view.
/// Re-run `retry` to pull one back into the resolution pipeline.
#[utoipa::path(
    post,
    path = "/api/v1/releases/{id}/keep",
    tag = "releases",
    params(("id" = String, Path, description = "Release id")),
    responses(
        (status = 200, body = ReleaseDto),
        (status = 404, description = "Release not found")
    ),
    security(("admin" = []))
)]
pub async fn keep(
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
        Some("standalone"),
        None,
        "standalone",
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
    let mut resolver = Resolver::new(
        state.db.clone(),
        state.metadata.clone(),
        state.ingestion.clone(),
    )
    .with_query_builder(state.query_builder.clone());
    if let Some(r) = state.mangaupdates_redirector.clone() {
        resolver = resolver.with_mangaupdates_redirector(r);
    }
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

/// Query parameters for `POST /releases/retry-all`.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(default, rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct RetryAllQuery {
    /// Also re-evaluate rows currently marked `resolved` (excluding
    /// manually-linked ones). Use after changing format-type rules,
    /// title-cleanup config, or the active provider's cache, when
    /// previously-confident matches need to be reconsidered against the
    /// new logic. `rejected`, `standalone`, and `manual`-path rows are
    /// always excluded.
    pub include_resolved: bool,
}

/// Re-run the resolver against every release currently visible in the
/// review queue (`unresolved`, `ambiguous`, `review_pending`). With
/// `?includeResolved=true`, also walks `resolved` rows (skipping
/// manually-linked ones). Spawns a background task and returns
/// immediately so the request doesn't block on what can be a multi-
/// minute walk. A dedicated per-process lock prevents a second click
/// from spawning a parallel walk; in that case the response is
/// `{ triggered: false, skipped: true }`.
#[utoipa::path(
    post,
    path = "/api/v1/releases/retry-all",
    tag = "releases",
    params(RetryAllQuery),
    responses((status = 202, body = RetryAllResponse)),
    security(("admin" = []))
)]
pub async fn retry_all(
    State(state): State<AppState>,
    Query(query): Query<RetryAllQuery>,
) -> ApiResult<Json<RetryAllResponse>> {
    let lock = state.locks.retry_all_releases_lock();
    let Ok(guard) = lock.try_lock_owned() else {
        return Ok(Json(RetryAllResponse {
            triggered: false,
            skipped: true,
        }));
    };

    let db = state.db.clone();
    let metadata = state.metadata.clone();
    let ingestion = state.ingestion.clone();
    let query_builder = state.query_builder.clone();
    let mu_redirector = state.mangaupdates_redirector.clone();
    let include_resolved = query.include_resolved;

    tokio::spawn(async move {
        let _g = guard;
        let mut resolver = Resolver::new(db, metadata, ingestion).with_query_builder(query_builder);
        if let Some(r) = mu_redirector {
            resolver = resolver.with_mangaupdates_redirector(r);
        }
        let result = if include_resolved {
            resolver.resolve_all(RETRY_ALL_BATCH_LIMIT).await
        } else {
            resolver.resolve_review_queue(RETRY_ALL_BATCH_LIMIT).await
        };
        match result {
            Ok(summary) => tracing::info!(
                include_resolved,
                resolved = summary.resolved,
                ambiguous = summary.ambiguous,
                review_pending = summary.review_pending,
                unresolved = summary.unresolved,
                errors = summary.errors,
                total = summary.total(),
                "retry-all batch completed"
            ),
            Err(e) => tracing::warn!(error = ?e, include_resolved, "retry-all batch failed"),
        }
    });

    Ok(Json(RetryAllResponse {
        triggered: true,
        skipped: false,
    }))
}

/// Bulk-reject a set of review-queue releases. The body's `ids` (or, when
/// empty, the filter fields) select the target set; every matched release is
/// pinned to `rejected` and its candidates dropped in one set-based update.
#[utoipa::path(
    post,
    path = "/api/v1/releases/bulk/reject",
    tag = "releases",
    request_body = BulkReviewRequest,
    responses((status = 200, body = BulkRejectResponse)),
    security(("admin" = []))
)]
pub async fn bulk_reject(
    State(state): State<AppState>,
    Json(req): Json<BulkReviewRequest>,
) -> ApiResult<Json<BulkRejectResponse>> {
    let ids = review_queue_target_ids(&state.db, &req, None).await?;
    let now = Utc::now().timestamp();
    let rejected = releases_repo::bulk_reject(&state.db, &ids, now)
        .await
        .map_err(anyhow_err)?;
    Ok(Json(BulkRejectResponse { rejected }))
}

/// Bulk-retry a set of review-queue releases. The body's `ids` (or, when
/// empty, the filter fields) select the target set (capped at the per-batch
/// limit). Spawns a background batch under the shared retry-all lock and
/// returns immediately; a concurrent retry-all / bulk-retry reports
/// `skipped: true`. An empty match set is reported as `triggered: false`
/// with `matched: 0` (no batch spawned, nothing skipped).
#[utoipa::path(
    post,
    path = "/api/v1/releases/bulk/retry",
    tag = "releases",
    request_body = BulkReviewRequest,
    responses((status = 202, body = BulkRetryResponse)),
    security(("admin" = []))
)]
pub async fn bulk_retry(
    State(state): State<AppState>,
    Json(req): Json<BulkReviewRequest>,
) -> ApiResult<Json<BulkRetryResponse>> {
    let ids = review_queue_target_ids(&state.db, &req, Some(RETRY_ALL_BATCH_LIMIT)).await?;
    let matched = ids.len() as u64;
    if ids.is_empty() {
        return Ok(Json(BulkRetryResponse {
            triggered: false,
            skipped: false,
            matched: 0,
        }));
    }

    let lock = state.locks.retry_all_releases_lock();
    let Ok(guard) = lock.try_lock_owned() else {
        return Ok(Json(BulkRetryResponse {
            triggered: false,
            skipped: true,
            matched,
        }));
    };

    let db = state.db.clone();
    let metadata = state.metadata.clone();
    let ingestion = state.ingestion.clone();
    let query_builder = state.query_builder.clone();
    let mu_redirector = state.mangaupdates_redirector.clone();

    tokio::spawn(async move {
        let _g = guard;
        let mut resolver = Resolver::new(db, metadata, ingestion).with_query_builder(query_builder);
        if let Some(r) = mu_redirector {
            resolver = resolver.with_mangaupdates_redirector(r);
        }
        match resolver.resolve_ids(&ids).await {
            Ok(summary) => tracing::info!(
                resolved = summary.resolved,
                ambiguous = summary.ambiguous,
                review_pending = summary.review_pending,
                unresolved = summary.unresolved,
                errors = summary.errors,
                total = summary.total(),
                "bulk-retry batch completed"
            ),
            Err(e) => tracing::warn!(error = ?e, "bulk-retry batch failed"),
        }
    });

    Ok(Json(BulkRetryResponse {
        triggered: true,
        skipped: false,
        matched,
    }))
}

/// Resolve a link request into the concrete `series.id` it targets, shared by
/// the single-release [`link`] handler and the [`bulk_link`] action. The
/// `posted_at` anchor is only consulted on the provider path, where it seeds
/// the freshly-upserted series' first-seen timestamp; callers pass the
/// release's own `posted_at` (single link) or the anchor release's (bulk).
/// Bulk-link a set of review-queue releases to one series. The body's `ids`
/// (intersected with the queue statuses) select the releases; the series is
/// named exactly as in the single-release [`link`] handler. Each release is
/// linked individually (clearing its candidates and bumping the series'
/// highest volume/chapter marks) so the result matches what N single links
/// would produce. The operator is responsible for the selection being one
/// series' releases; the endpoint links them blindly to the chosen target.
#[utoipa::path(
    post,
    path = "/api/v1/releases/bulk/link",
    tag = "releases",
    request_body = BulkLinkRequest,
    responses(
        (status = 200, body = BulkLinkResponse),
        (status = 400, description = "Empty ids, malformed target, or unknown provider/external_id"),
        (status = 404, description = "Target series not found")
    ),
    security(("admin" = []))
)]
pub async fn bulk_link(
    State(state): State<AppState>,
    Json(req): Json<BulkLinkRequest>,
) -> ApiResult<Json<BulkLinkResponse>> {
    if req.ids.is_empty() {
        return Err(ApiError::BadRequest("`ids` must not be empty".into()));
    }
    // Intersect with the queue statuses so a since-decided release can't be
    // relinked out from under another decision.
    let target_req = BulkReviewRequest {
        ids: req.ids.clone(),
        ..Default::default()
    };
    let ids = review_queue_target_ids(&state.db, &target_req, None).await?;

    let now = Utc::now();
    let attempted_at = now.timestamp();
    // The anchor only matters for the provider path (it seeds a freshly
    // upserted series' first-seen time). Use the first targeted release's
    // `posted_at`; fall back to now when nothing matched.
    let anchor_posted_at = match ids.first() {
        Some(first) => releases_repo::find_by_id(&state.db, first)
            .await
            .map_err(anyhow_err)?
            .map(|r| r.posted_at)
            .unwrap_or(attempted_at),
        None => attempted_at,
    };
    let link_req = LinkRequest {
        series_id: req.series_id,
        provider: req.provider,
        external_id: req.external_id,
    };
    let series_id = resolve_link_target(&state, anchor_posted_at, link_req, now).await?;

    let mut linked = 0u64;
    for id in &ids {
        review_repo::replace_for_release(&state.db, id, Vec::new())
            .await
            .map_err(anyhow_err)?;
        persist::link_release(
            &state.db,
            id,
            Some(series_id),
            Some("manual"),
            Some(1.0),
            "resolved",
            attempted_at,
        )
        .await
        .map_err(ApiError::Internal)?;
        linked += 1;
    }

    Ok(Json(BulkLinkResponse { linked, series_id }))
}

async fn resolve_link_target(
    state: &AppState,
    posted_at: i64,
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
            // Operator picked a (provider, external_id) for a release that
            // didn't have a local series yet. Use the conservative default:
            // if the foreign-id chain happens to land on a pre-existing
            // manual row, don't clobber it.
            Ok(persist::upsert_series_from_metadata(
                &state.db, &provider, &metadata, posted_at, now, false,
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

pub(crate) fn model_to_release(m: releases::Model, formats: Vec<String>) -> ReleaseDto {
    let files = m
        .files_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default();
    let extracted_links = parse_extracted_links(m.extracted_links_json.as_deref());
    let comment_suggested_links = parse_extracted_links(m.comment_suggested_links_json.as_deref());
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
        description_html: m.description_html,
        extracted_links,
        comment_suggested_links,
        information_url: m.information_url,
        sent_to_client_at: m.sent_to_client_at,
        sent_to_client_label: m.sent_to_client_label,
    }
}

/// Decode `releases.extracted_links_json` into the API DTO. Returns `None`
/// when the column is null, the JSON is malformed, or every field inside
/// is empty — so the consumer can render "no links found" without
/// defensively checking each provider.
fn parse_extracted_links(raw: Option<&str>) -> Option<ExtractedLinksDto> {
    let raw = raw?;
    let dto: ExtractedLinksDto = serde_json::from_str(raw).ok()?;
    if dto.is_empty() { None } else { Some(dto) }
}

fn anyhow_err<E: Into<anyhow::Error>>(e: E) -> ApiError {
    ApiError::Internal(e.into())
}
