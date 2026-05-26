//! Discovery-source listing + manual-poll trigger.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use td_config::SourceConfig;
use td_db::repos::sources_repo;
use td_scheduler::jobs::poll_source;
use utoipa::ToSchema;

use crate::errors::{ApiError, ApiResult};
use crate::state::{AppState, JobEvent, JobKind, JobResult};

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
        items.push(SourceDto {
            name: name.to_string(),
            kind: source.kind().to_string(),
            last_polled_at: row.as_ref().and_then(|r| r.last_polled_at),
            last_success_at: row.as_ref().and_then(|r| r.last_success_at),
            last_error: row.as_ref().and_then(|r| r.last_error.clone()),
            last_summary: row.as_ref().and_then(|r| r.last_summary.clone()),
            config,
        });
    }
    // Stable ordering for snapshot tests / UI sort.
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(SourceList { items }))
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

    // Check the lock optimistically: if another tick is in flight we want
    // to report `skipped` rather than spawn a task that will silently
    // bail. `run_tick` itself does the same `try_lock` dance.
    let lock = state.locks.source_lock(&name);
    let skipped = lock.try_lock().is_err();
    if skipped {
        // Single `finished{skipped:true}` so observers see the no-op
        // without a phantom `started` first.
        state.send_job_event(JobEvent::finished(
            JobKind::Source,
            &name,
            JobResult {
                triggered: false,
                skipped: true,
                ..Default::default()
            },
        ));
        return Ok(Json(ManualPollResponse {
            source: name,
            triggered: false,
            skipped: true,
        }));
    }
    // Drop the test-lock; the spawned tick will re-acquire it. This is
    // racy in theory (another tick could grab it between the drop and the
    // spawn), but at worst the spawned tick will skip itself — which is
    // exactly the desired behaviour anyway.

    state.send_job_event(JobEvent::started(JobKind::Source, &name));

    let db = state.db.clone();
    let metadata = state.metadata.clone();
    let ingestion = state.ingestion.clone();
    let locks = state.locks.clone();
    let query_builder = state.query_builder.clone();
    let mu_redirector = state.mangaupdates_redirector.clone();
    let events = state.job_events.clone();
    let event_name = name.clone();
    tokio::spawn(async move {
        poll_source::run_tick(
            source,
            db,
            metadata,
            ingestion,
            locks,
            query_builder,
            mu_redirector,
            "manual",
        )
        .await;
        let _ = events.send(JobEvent::finished(
            JobKind::Source,
            event_name,
            JobResult {
                triggered: true,
                skipped: false,
                ..Default::default()
            },
        ));
    });

    Ok(Json(ManualPollResponse {
        source: name,
        triggered: true,
        skipped: false,
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
        if lock.try_lock().is_err() {
            state.send_job_event(JobEvent::finished(
                JobKind::Source,
                &name,
                JobResult {
                    triggered: false,
                    skipped: true,
                    ..Default::default()
                },
            ));
            results.push(ManualPollResponse {
                source: name,
                triggered: false,
                skipped: true,
            });
            continue;
        }
        state.send_job_event(JobEvent::started(JobKind::Source, &name));
        let db = state.db.clone();
        let metadata = state.metadata.clone();
        let ingestion = state.ingestion.clone();
        let locks = state.locks.clone();
        let query_builder = state.query_builder.clone();
        let mu_redirector = state.mangaupdates_redirector.clone();
        let spawned: Arc<dyn td_source::DiscoverySource> = source;
        let events = state.job_events.clone();
        let event_name = name.clone();
        tokio::spawn(async move {
            poll_source::run_tick(
                spawned,
                db,
                metadata,
                ingestion,
                locks,
                query_builder,
                mu_redirector,
                "manual",
            )
            .await;
            let _ = events.send(JobEvent::finished(
                JobKind::Source,
                event_name,
                JobResult {
                    triggered: true,
                    skipped: false,
                    ..Default::default()
                },
            ));
        });
        results.push(ManualPollResponse {
            source: name,
            triggered: true,
            skipped: false,
        });
    }

    Ok(Json(PollAllResponse { results }))
}
