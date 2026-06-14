//! Discovery-source listing + manual poll / backfill triggers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use td_config::SourceConfig;
use td_db::repos::{releases_repo, run_metrics_repo, sources_repo};
use td_scheduler::dispatch;
use td_scheduler::jobs::backfill_source;
use td_scheduler::jobs::poll_source;
use td_scheduler::jobs::reenrich_source;
use utoipa::{IntoParams, ToSchema};

use crate::errors::{ApiError, ApiResult};
use crate::handlers::tagging::{TagList, TagUsageDto};
use crate::state::{AppState, InFlight, JobKind, JobResult};

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceDto {
    pub name: String,
    pub kind: String,
    pub last_polled_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_error: Option<String>,
    pub last_summary: Option<String>,
    /// Static config block snapshotted at boot. `None` when a source is in
    /// the registry but its config entry isn't, which today only happens in
    /// tests using a hand-built `SourceRegistry`.
    pub config: Option<SourceConfigDto>,
    /// Set when a `poll_runs` row for this source is currently in
    /// `status = 'running'`. Lets the admin UI render the "RUNNING…" pill
    /// from a fresh page load instead of waiting for an SSE event the
    /// channel doesn't replay.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_flight: Option<InFlight>,
}

/// Operator-facing snapshot of the per-source config (the bits visible in
/// `[[sources]]` and any kind-specific nested options). Never carries
/// secrets; the source layer doesn't have any in v1.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceConfigDto {
    pub enabled: bool,
    pub cron: Option<String>,
    /// Feed URL for the source kind. Empty when the kind doesn't define one
    /// (every kind today does, so this is informational future-proofing).
    pub feed_url: String,
    pub fetch_details: bool,
    pub timeout_seconds: u32,
    /// Override for the site base URL. Useful when the feed is proxied.
    pub site_base_url: Option<String>,
    /// Maximum number of feed pages walked per poll. Always `1` today:
    /// no v1 source kind paginates inside the steady-state poll. Kept on
    /// the DTO as a placeholder for future paginated source kinds; the
    /// `backfill` CLI is the historical-catch-up path in the meantime.
    pub max_pages: u32,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceList {
    pub items: Vec<SourceDto>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManualPollResponse {
    pub source: String,
    pub triggered: bool,
    /// `false` when a previous tick is still in flight; the request is a no-op.
    pub skipped: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PollAllResponse {
    pub results: Vec<ManualPollResponse>,
}

fn build_config_dto(cfg: &SourceConfig) -> SourceConfigDto {
    // For v1 only the nyaa kind exists, so we read directly from it. New
    // kinds add an arm here.
    let (feed_url, fetch_details, timeout_seconds, site_base_url) = match cfg.kind.as_str() {
        "nyaa" => {
            let opts = cfg.nyaa.as_ref();
            (
                opts.map(|o| o.feed_url.clone()).unwrap_or_default(),
                opts.map(|o| o.fetch_details).unwrap_or(true),
                opts.map(|o| o.timeout_seconds).unwrap_or(30),
                opts.map(|o| o.site_base_url.clone()),
            )
        }
        _ => (String::new(), true, 30, None),
    };
    SourceConfigDto {
        enabled: cfg.enabled,
        cron: cfg.cron.clone(),
        feed_url,
        fetch_details,
        timeout_seconds,
        site_base_url,
        max_pages: 1,
    }
}

fn find_source_config<'a>(configs: &'a [SourceConfig], name: &str) -> Option<&'a SourceConfig> {
    configs.iter().find(|c| c.name == name)
}

/// List every registered discovery source with its last-poll markers.
#[utoipa::path(
    get,
    path = "/api/v1/sources",
    tag = "sources",
    operation_id = "list_sources",
    responses((status = 200, body = SourceList))
)]
pub async fn list(State(state): State<AppState>) -> ApiResult<Json<SourceList>> {
    let mut items = Vec::with_capacity(state.sources.len());
    for (name, source) in state.sources.iter() {
        let row = sources_repo::get(&state.db, source.kind(), name)
            .await
            .map_err(ApiError::Internal)?;
        let config = find_source_config(&state.sources_config, name).map(build_config_dto);
        let in_flight = run_metrics_repo::find_in_flight_poll_for_source(&state.db, name)
            .await
            .map_err(ApiError::Internal)?
            .map(InFlight::from_row);
        items.push(SourceDto {
            name: name.to_string(),
            kind: source.kind().to_string(),
            last_polled_at: row.as_ref().and_then(|r| r.last_polled_at),
            last_success_at: row.as_ref().and_then(|r| r.last_success_at),
            last_error: row.as_ref().and_then(|r| r.last_error.clone()),
            last_summary: row.as_ref().and_then(|r| r.last_summary.clone()),
            config,
            in_flight,
        });
    }
    // Stable ordering for snapshot tests / UI sort.
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(SourceList { items }))
}

/// Discovery sources that have linked at least one release, with the count of
/// distinct series each resolved to (sorted by descending count, then name).
/// Powers the admin-only source filter's dropdown. Admin-only (lives in the
/// `require_admin` group) since the filter it feeds is itself admin-only —
/// there's no reason to expose the operator's feed names on a public read.
#[utoipa::path(
    get,
    path = "/api/v1/sources/with-series-count",
    tag = "sources",
    operation_id = "list_sources_with_series_count",
    responses((status = 200, body = TagList)),
    security(("admin" = []))
)]
pub async fn list_with_series_counts(State(state): State<AppState>) -> ApiResult<Json<TagList>> {
    let rows = releases_repo::list_sources_with_series_counts(&state.db)
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

/// Trigger a one-shot poll for the named source. Uses the same per-source
/// mutex the cron job holds, so a manual kick during a scheduled tick is
/// silently skipped (`skipped = true`).
#[utoipa::path(
    post,
    path = "/api/v1/sources/{name}/poll",
    tag = "sources",
    params(("name" = String, Path, description = "Source instance name")),
    responses(
        (status = 202, body = ManualPollResponse),
        (status = 404, description = "No source with that name registered")
    ),
    security(("admin" = []))
)]
pub async fn poll(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<Json<ManualPollResponse>> {
    let source = state
        .sources
        .get(&name)
        .cloned()
        .ok_or_else(|| ApiError::NotFound(format!("source {name:?}")))?;

    let lock = state.locks.source_lock(&name);
    let db = state.db.clone();
    let metadata = state.metadata.clone();
    let ingestion = state.ingestion.clone();
    let query_builder = state.query_builder.clone();
    let mu_redirector = state.mangaupdates_redirector.clone();
    let events = state.job_events.clone();
    let started_at_ts = chrono::Utc::now().timestamp();
    let db_for_skip = state.db.clone();
    let name_for_skip = name.clone();
    let kind_for_skip = source.kind().to_string();
    let triggered = dispatch::try_dispatch(
        &state.job_events,
        lock,
        JobKind::Source,
        name.clone(),
        move || async move {
            poll_source::record_skipped(
                &db_for_skip,
                &name_for_skip,
                &kind_for_skip,
                started_at_ts,
                run_metrics_repo::trigger::MANUAL,
            )
            .await;
        },
        move || async move {
            poll_source::run_tick(
                source,
                db,
                metadata,
                ingestion,
                query_builder,
                mu_redirector,
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

    Ok(Json(ManualPollResponse {
        source: name,
        triggered,
        skipped: !triggered,
    }))
}

/// Fan-out trigger for every registered source. Returns a per-source
/// triggered/skipped result without aggregating: callers see the same
/// per-source outcomes a series of single-source calls would have produced.
#[utoipa::path(
    post,
    path = "/api/v1/sources/poll-all",
    tag = "sources",
    responses((status = 202, body = PollAllResponse)),
    security(("admin" = []))
)]
pub async fn poll_all(State(state): State<AppState>) -> ApiResult<Json<PollAllResponse>> {
    let mut names: Vec<String> = state.sources.names().map(|n| n.to_string()).collect();
    // Stable order matches `list` so the UI can zip the two responses.
    names.sort();

    let mut results = Vec::with_capacity(names.len());
    for name in names {
        let Some(source) = state.sources.get(&name).cloned() else {
            // Race: a source got removed mid-iteration. Skip silently.
            continue;
        };
        let lock = state.locks.source_lock(&name);
        let db = state.db.clone();
        let metadata = state.metadata.clone();
        let ingestion = state.ingestion.clone();
        let query_builder = state.query_builder.clone();
        let mu_redirector = state.mangaupdates_redirector.clone();
        let events = state.job_events.clone();
        let spawned: Arc<dyn td_source::DiscoverySource> = source;
        let started_at_ts = chrono::Utc::now().timestamp();
        let db_for_skip = state.db.clone();
        let name_for_skip = name.clone();
        let kind_for_skip = spawned.kind().to_string();
        let triggered = dispatch::try_dispatch(
            &state.job_events,
            lock,
            JobKind::Source,
            name.clone(),
            move || async move {
                poll_source::record_skipped(
                    &db_for_skip,
                    &name_for_skip,
                    &kind_for_skip,
                    started_at_ts,
                    run_metrics_repo::trigger::MANUAL,
                )
                .await;
            },
            move || async move {
                poll_source::run_tick(
                    spawned,
                    db,
                    metadata,
                    ingestion,
                    query_builder,
                    mu_redirector,
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
        results.push(ManualPollResponse {
            source: name,
            triggered,
            skipped: !triggered,
        });
    }

    Ok(Json(PollAllResponse { results }))
}

/// Default `pages` when the query param is omitted. Matches the `tsundoku
/// backfill` CLI default: a single page, the cheapest useful catch-up.
fn default_backfill_pages() -> u32 {
    1
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct BackfillParams {
    /// Number of listing pages to walk, starting at page 1. Clamped to a
    /// minimum of 1.
    #[serde(default = "default_backfill_pages")]
    pub pages: u32,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManualBackfillResponse {
    pub source: String,
    /// Pages the run was asked to walk (after clamping); it may stop early
    /// if the source runs out of history.
    pub pages: u32,
    pub triggered: bool,
    /// `true` when a poll or backfill for this source was already in
    /// flight; the request is a no-op.
    pub skipped: bool,
}

/// Trigger an in-process historical backfill for the named source. Runs
/// the same loop as the `tsundoku backfill` CLI, but inside the serve
/// process under the shared per-source mutex, so it cannot race a cron
/// poll (returns `skipped = true` when work is already in flight). Returns
/// `422` when the source's kind does not support backfill.
#[utoipa::path(
    post,
    path = "/api/v1/sources/{name}/backfill",
    tag = "sources",
    params(
        ("name" = String, Path, description = "Source instance name"),
        BackfillParams,
    ),
    responses(
        (status = 202, body = ManualBackfillResponse),
        (status = 404, description = "No source with that name registered"),
        (status = 422, description = "Source kind does not support historical backfill")
    ),
    security(("admin" = []))
)]
pub async fn backfill(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(params): Query<BackfillParams>,
) -> ApiResult<Json<ManualBackfillResponse>> {
    let pages = params.pages.max(1);
    let source = state
        .sources
        .get(&name)
        .cloned()
        .ok_or_else(|| ApiError::NotFound(format!("source {name:?}")))?;

    // Reject non-backfillable kinds up front, before touching the lock, so
    // the caller gets a capability error rather than a misleading 202.
    if source.as_backfillable().is_none() {
        return Err(ApiError::BadRequest(format!(
            "source {name:?} (kind={}) does not support historical backfill",
            source.kind()
        )));
    }

    let lock = state.locks.source_lock(&name);
    let db = state.db.clone();
    let metadata = state.metadata.clone();
    let ingestion = state.ingestion.clone();
    let query_builder = state.query_builder.clone();
    let mu_redirector = state.mangaupdates_redirector.clone();
    let events = state.job_events.clone();
    let event_name = name.clone();
    let started_at_ts = chrono::Utc::now().timestamp();
    let db_for_skip = state.db.clone();
    let name_for_skip = name.clone();
    let kind_for_skip = source.kind().to_string();
    let triggered = dispatch::try_dispatch(
        &state.job_events,
        lock,
        JobKind::Source,
        name.clone(),
        move || async move {
            poll_source::record_skipped(
                &db_for_skip,
                &name_for_skip,
                &kind_for_skip,
                started_at_ts,
                run_metrics_repo::trigger::MANUAL,
            )
            .await;
        },
        move || async move {
            let result = backfill_source::run(
                source,
                db,
                metadata,
                ingestion,
                query_builder,
                mu_redirector,
                events,
                pages,
                run_metrics_repo::trigger::MANUAL,
            )
            .await;
            let new = match &result {
                Ok(totals) => Some(totals.new as i64),
                Err(_) => None,
            };
            if let Err(e) = &result {
                tracing::warn!(error = ?e, source = %event_name, "manual backfill failed");
            }
            JobResult {
                triggered: true,
                skipped: false,
                new,
                ..Default::default()
            }
        },
    );

    Ok(Json(ManualBackfillResponse {
        source: name,
        pages,
        triggered,
        skipped: !triggered,
    }))
}

/// The resolution statuses a release can carry. A re-enrich request's
/// `statuses` must be a subset of these; an unknown value is a client error
/// rather than a silent no-op.
const VALID_STATUSES: &[&str] = &[
    "unresolved",
    "ambiguous",
    "review_pending",
    "resolved",
    "standalone",
    "rejected",
];

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReenrichRequest {
    /// Resolution statuses to target. Releases for the source whose
    /// `resolutionStatus` is in this set have their detail page re-fetched
    /// and their source-derived columns refreshed; resolution state is left
    /// untouched. Must be non-empty and a subset of the known statuses.
    pub statuses: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManualReenrichResponse {
    pub source: String,
    /// The (validated) statuses the run targets.
    pub statuses: Vec<String>,
    pub triggered: bool,
    /// `true` when a poll / backfill / re-enrich for this source was already
    /// in flight; the request is a no-op.
    pub skipped: bool,
}

/// Re-fetch the detail page for the source's already-persisted releases that
/// match `statuses`, refreshing their source-derived columns (files,
/// description, extracted links, information URL) without touching resolution
/// state. Use after a parser change adds or fixes a detail-page field.
/// Shares the per-source mutex, so it cannot race a cron poll / backfill
/// (returns `skipped = true` when work is already in flight).
#[utoipa::path(
    post,
    path = "/api/v1/sources/{name}/re-enrich",
    tag = "sources",
    params(("name" = String, Path, description = "Source instance name")),
    request_body = ReenrichRequest,
    responses(
        (status = 202, body = ManualReenrichResponse),
        (status = 400, description = "Empty or unknown status in the request"),
        (status = 404, description = "No source with that name registered")
    ),
    security(("admin" = []))
)]
pub async fn reenrich(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<ReenrichRequest>,
) -> ApiResult<Json<ManualReenrichResponse>> {
    let source = state
        .sources
        .get(&name)
        .cloned()
        .ok_or_else(|| ApiError::NotFound(format!("source {name:?}")))?;

    if req.statuses.is_empty() {
        return Err(ApiError::BadRequest(
            "statuses must not be empty".to_string(),
        ));
    }
    // De-dupe while preserving order, and reject anything not in the known
    // set so a typo surfaces as a 400 instead of selecting zero rows.
    let mut statuses: Vec<String> = Vec::with_capacity(req.statuses.len());
    for s in req.statuses {
        if !VALID_STATUSES.contains(&s.as_str()) {
            return Err(ApiError::BadRequest(format!("unknown status {s:?}")));
        }
        if !statuses.contains(&s) {
            statuses.push(s);
        }
    }

    let lock = state.locks.source_lock(&name);
    let db = state.db.clone();
    let events = state.job_events.clone();
    let event_name = name.clone();
    let job_statuses = statuses.clone();
    let started_at_ts = chrono::Utc::now().timestamp();
    let db_for_skip = state.db.clone();
    let name_for_skip = name.clone();
    let kind_for_skip = source.kind().to_string();
    let triggered = dispatch::try_dispatch(
        &state.job_events,
        lock,
        JobKind::Source,
        name.clone(),
        move || async move {
            poll_source::record_skipped(
                &db_for_skip,
                &name_for_skip,
                &kind_for_skip,
                started_at_ts,
                run_metrics_repo::trigger::MANUAL,
            )
            .await;
        },
        move || async move {
            let result = reenrich_source::run(
                source,
                db,
                job_statuses,
                events,
                run_metrics_repo::trigger::MANUAL,
            )
            .await;
            if let Err(e) = &result {
                tracing::warn!(error = ?e, source = %event_name, "manual re-enrich failed");
            }
            JobResult {
                triggered: true,
                skipped: false,
                ..Default::default()
            }
        },
    );

    Ok(Json(ManualReenrichResponse {
        source: name,
        statuses,
        triggered,
        skipped: !triggered,
    }))
}
