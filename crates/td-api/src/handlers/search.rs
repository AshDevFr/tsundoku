//! Per-series release-search endpoints: list the configured `[[search]]`
//! entries, trigger a search for one series, and read a series' run
//! history.
//!
//! All three live in the admin write group (the entries list included: it
//! leaks config naming). The trigger follows the manual-poll idiom: the
//! walk runs detached under the per-entry `search:<name>` lock via
//! `try_dispatch`, and a concurrent trigger reports `skipped` instead of
//! stacking a second walk against the upstream. Liveness is polled from
//! `search_runs` (the walk inserts a `running` row), not via SSE.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use td_db::repos::{search_runs_repo, series_repo};
use td_scheduler::jobs::search_series;
use td_scheduler::{JobKind, JobResult, dispatch};
use utoipa::{IntoParams, ToSchema};

use crate::errors::{ApiError, ApiResult};
use crate::state::AppState;

/// One configured `[[search]]` endpoint, with the display fields the
/// admin Sources page renders. Disabled entries are never listed (they
/// aren't registered).
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchEntryDto {
    pub name: String,
    pub kind: String,
    /// The split button's primary action. Exactly one listed entry is
    /// `true` (explicitly marked, or the first entry as fallback).
    #[serde(rename = "default")]
    pub is_default: bool,
    /// Listing URL the search appends `q`/`p` to (filters baked in).
    pub search_url: String,
    pub max_pages: u32,
    pub fetch_details: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchEntriesResponse {
    pub items: Vec<SearchEntryDto>,
}

/// The configured release-search endpoints, in config (dropdown) order.
#[utoipa::path(
    get,
    path = "/api/v1/search/entries",
    tag = "search",
    responses((status = 200, body = SearchEntriesResponse)),
    security(("admin" = []))
)]
pub async fn entries(State(state): State<AppState>) -> ApiResult<Json<SearchEntriesResponse>> {
    let default_name = state
        .search
        .default_entry()
        .map(|e| e.source.name().to_string());
    let items = state
        .search
        .iter()
        .map(|entry| {
            let name = entry.source.name();
            // Display fields come from the raw config snapshot; the
            // registry's trait objects deliberately don't expose them.
            let (search_url, fetch_details) = state
                .search_config
                .iter()
                .find(|c| c.name == name)
                .and_then(|c| c.nyaa.as_ref())
                .map(|o| (o.search_url.clone(), o.fetch_details))
                .unwrap_or_default();
            SearchEntryDto {
                name: name.to_string(),
                kind: entry.source.kind().to_string(),
                is_default: default_name.as_deref() == Some(name),
                search_url,
                max_pages: entry.max_pages,
                fetch_details,
            }
        })
        .collect();
    Ok(Json(SearchEntriesResponse { items }))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchReleasesRequest {
    /// `[[search]]` entry name. Omitted ⇒ the default entry.
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchReleasesResponse {
    /// Entry the walk was (or would have been) dispatched against.
    pub search: String,
    pub series_id: i32,
    pub triggered: bool,
    /// `true` when a walk against this entry was already in flight.
    pub skipped: bool,
}

/// Trigger a release search for one series. The walk runs in the
/// background; poll `GET /series/{id}/search-runs` for completion.
#[utoipa::path(
    post,
    path = "/api/v1/series/{id}/search-releases",
    tag = "search",
    params(("id" = i32, Path, description = "Series id")),
    request_body = SearchReleasesRequest,
    responses(
        (status = 202, body = SearchReleasesResponse),
        (status = 400, description = "Unknown or disabled search entry"),
        (status = 404, description = "No series with that id"),
        (status = 503, description = "No [[search]] entries configured")
    ),
    security(("admin" = []))
)]
pub async fn trigger(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<SearchReleasesRequest>,
) -> ApiResult<Json<SearchReleasesResponse>> {
    series_repo::find_by_id(&state.db, id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::NotFound(format!("series {id}")))?;

    if state.search.is_empty() {
        return Err(ApiError::Misconfigured(
            "no [[search]] entries configured; add one to the config file".into(),
        ));
    }
    let entry = match &req.search {
        Some(name) => state.search.get(name).ok_or_else(|| {
            ApiError::BadRequest(format!("unknown search entry {name:?} (or it is disabled)"))
        })?,
        None => state
            .search
            .default_entry()
            .expect("non-empty registry always has a default entry"),
    };

    let entry_name = entry.source.name().to_string();
    let lock = state.locks.search_lock(&entry_name);
    let source = entry.source.clone();
    let max_pages = entry.max_pages;
    let db = state.db.clone();
    let metadata = state.metadata.clone();
    let ingestion = state.ingestion.clone();
    let query_builder = state.query_builder.clone();
    let mu_redirector = state.mangaupdates_redirector.clone();
    let triggered = dispatch::try_dispatch(
        &state.job_events,
        lock,
        JobKind::Search,
        entry_name.clone(),
        // Skips are visible in the HTTP response itself; there is no
        // metrics lane to write a "skipped" row into.
        || async {},
        move || async move {
            // Errors are already recorded on the search_runs row (or
            // logged for setup faults); nothing to propagate from a
            // detached task.
            if let Err(e) = search_series::run(
                source,
                max_pages,
                db,
                metadata,
                ingestion,
                query_builder,
                mu_redirector,
                id,
                search_runs_repo::TRIGGER_MANUAL,
            )
            .await
            {
                tracing::error!(error = ?e, series_id = id, "series search failed at setup");
            }
            JobResult {
                triggered: true,
                skipped: false,
                ..Default::default()
            }
        },
    );

    Ok(Json(SearchReleasesResponse {
        search: entry_name,
        series_id: id,
        triggered,
        skipped: !triggered,
    }))
}

/// Body for `POST /api/v1/series/bulk/search-releases`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BulkSearchReleasesRequest {
    /// Series ids to search for. Must be non-empty; ids with no series row
    /// are dropped from `matched` (the whole batch 404s only when none
    /// exist).
    pub ids: Vec<i32>,
    /// `[[search]]` entry name. Omitted ⇒ the default entry.
    #[serde(default)]
    pub search: Option<String>,
}

/// Response for `POST /api/v1/series/bulk/search-releases`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BulkSearchReleasesResponse {
    /// Entry the walks were (or would have been) dispatched against.
    pub search: String,
    /// Number of listed ids that exist and were queued for a walk.
    pub matched: u64,
    pub triggered: bool,
    /// `true` when a walk against this entry was already in flight — the
    /// whole batch is skipped, nothing partial runs.
    pub skipped: bool,
}

/// Trigger a release search for a whole selection of series.
///
/// One dispatch against the entry's `search:<name>` lock; the detached job
/// walks the series **sequentially**, so upstream rate-limiting sees one walk
/// at a time and each series still gets its own `search_runs` audit row
/// (feeding the run-history timelines). Poll `GET /series/{id}/search-runs`
/// per series for completion, same as the single trigger.
#[utoipa::path(
    post,
    path = "/api/v1/series/bulk/search-releases",
    tag = "search",
    request_body = BulkSearchReleasesRequest,
    responses(
        (status = 200, body = BulkSearchReleasesResponse),
        (status = 400, description = "Empty ids list, or unknown/disabled search entry"),
        (status = 404, description = "None of the listed series exist"),
        (status = 503, description = "No [[search]] entries configured")
    ),
    security(("admin" = []))
)]
pub async fn bulk_trigger(
    State(state): State<AppState>,
    Json(req): Json<BulkSearchReleasesRequest>,
) -> ApiResult<Json<BulkSearchReleasesResponse>> {
    if req.ids.is_empty() {
        return Err(ApiError::BadRequest("ids must not be empty".into()));
    }
    if state.search.is_empty() {
        return Err(ApiError::Misconfigured(
            "no [[search]] entries configured; add one to the config file".into(),
        ));
    }
    let entry = match &req.search {
        Some(name) => state.search.get(name).ok_or_else(|| {
            ApiError::BadRequest(format!("unknown search entry {name:?} (or it is disabled)"))
        })?,
        None => state
            .search
            .default_entry()
            .expect("non-empty registry always has a default entry"),
    };

    let mut existing = Vec::with_capacity(req.ids.len());
    for id in &req.ids {
        if series_repo::find_by_id(&state.db, *id)
            .await
            .map_err(ApiError::from)?
            .is_some()
        {
            existing.push(*id);
        }
    }
    if existing.is_empty() {
        return Err(ApiError::NotFound("none of the listed series exist".into()));
    }
    let matched = existing.len() as u64;

    let entry_name = entry.source.name().to_string();
    let lock = state.locks.search_lock(&entry_name);
    let source = entry.source.clone();
    let max_pages = entry.max_pages;
    let db = state.db.clone();
    let metadata = state.metadata.clone();
    let ingestion = state.ingestion.clone();
    let query_builder = state.query_builder.clone();
    let mu_redirector = state.mangaupdates_redirector.clone();
    let triggered = dispatch::try_dispatch(
        &state.job_events,
        lock,
        JobKind::Search,
        entry_name.clone(),
        // Skips are visible in the HTTP response itself; there is no
        // metrics lane to write a "skipped" row into.
        || async {},
        move || async move {
            for id in existing {
                // Per-series errors are already recorded on the series'
                // search_runs row (or logged for setup faults); keep
                // walking the rest of the batch.
                if let Err(e) = search_series::run(
                    source.clone(),
                    max_pages,
                    db.clone(),
                    metadata.clone(),
                    ingestion.clone(),
                    query_builder.clone(),
                    mu_redirector.clone(),
                    id,
                    search_runs_repo::TRIGGER_MANUAL,
                )
                .await
                {
                    tracing::error!(error = ?e, series_id = id, "series search failed at setup");
                }
            }
            JobResult {
                triggered: true,
                skipped: false,
                ..Default::default()
            }
        },
    );

    Ok(Json(BulkSearchReleasesResponse {
        search: entry_name,
        matched,
        triggered,
        skipped: !triggered,
    }))
}

/// One `search_runs` row. `outcome` is `running` | `success` | `error`;
/// counts are set on completion only.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchRunDto {
    pub id: i64,
    pub ran_at: i64,
    pub finished_at: Option<i64>,
    pub search_name: String,
    pub series_id: i32,
    pub trigger: String,
    pub outcome: String,
    pub queries_attempted: Option<i64>,
    pub pages_fetched: Option<i64>,
    pub releases_seen: Option<i64>,
    pub releases_new: Option<i64>,
    pub error: Option<String>,
}

impl From<search_runs_repo::Model> for SearchRunDto {
    fn from(m: search_runs_repo::Model) -> Self {
        Self {
            id: m.id,
            ran_at: m.ran_at,
            finished_at: m.finished_at,
            search_name: m.search_name,
            series_id: m.series_id,
            trigger: m.trigger,
            outcome: m.outcome,
            queries_attempted: m.queries_attempted,
            pages_fetched: m.pages_fetched,
            releases_seen: m.releases_seen,
            releases_new: m.releases_new,
            error: m.error,
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct SearchRunsQuery {
    /// Maximum rows to return (newest first). Defaults to 10.
    pub limit: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchRunsResponse {
    pub items: Vec<SearchRunDto>,
}

/// Recent search runs for one series, newest first. The newest row's
/// `outcome = running` is the "search in flight" signal the UI polls.
#[utoipa::path(
    get,
    path = "/api/v1/series/{id}/search-runs",
    tag = "search",
    operation_id = "series_search_runs",
    params(("id" = i32, Path, description = "Series id"), SearchRunsQuery),
    responses(
        (status = 200, body = SearchRunsResponse),
        (status = 404, description = "No series with that id")
    ),
    security(("admin" = []))
)]
pub async fn runs(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Query(q): Query<SearchRunsQuery>,
) -> ApiResult<Json<SearchRunsResponse>> {
    series_repo::find_by_id(&state.db, id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::NotFound(format!("series {id}")))?;
    let limit = q.limit.unwrap_or(10).clamp(1, 100);
    let items = search_runs_repo::recent_for_series(&state.db, id, limit)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(SearchRunDto::from)
        .collect();
    Ok(Json(SearchRunsResponse { items }))
}

/// One run for the global timeline: a [`SearchRunDto`] plus the series'
/// canonical title so the admin list can link each row to its series.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchRunDto {
    #[serde(flatten)]
    pub run: SearchRunDto,
    pub series_title: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchRunsResponse {
    pub items: Vec<GlobalSearchRunDto>,
}

/// Recent search runs across every series, newest first, for the admin
/// Sources page's global timeline.
#[utoipa::path(
    get,
    path = "/api/v1/search/runs",
    tag = "search",
    operation_id = "global_search_runs",
    params(SearchRunsQuery),
    responses((status = 200, body = GlobalSearchRunsResponse)),
    security(("admin" = []))
)]
pub async fn global_runs(
    State(state): State<AppState>,
    Query(q): Query<SearchRunsQuery>,
) -> ApiResult<Json<GlobalSearchRunsResponse>> {
    let limit = q.limit.unwrap_or(10).clamp(1, 50);
    let items = search_runs_repo::recent_all(&state.db, limit)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(|r| GlobalSearchRunDto {
            run: SearchRunDto {
                id: r.id,
                ran_at: r.ran_at,
                finished_at: r.finished_at,
                search_name: r.search_name,
                series_id: r.series_id,
                trigger: r.trigger,
                outcome: r.outcome,
                queries_attempted: r.queries_attempted,
                pages_fetched: r.pages_fetched,
                releases_seen: r.releases_seen,
                releases_new: r.releases_new,
                error: r.error,
            },
            series_title: r.series_title,
        })
        .collect();
    Ok(Json(GlobalSearchRunsResponse { items }))
}
