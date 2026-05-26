//! Discovery-source listing + manual-poll trigger.

use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use td_db::repos::sources_repo;
use td_scheduler::jobs::poll_source;
use utoipa::ToSchema;

use crate::errors::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceDto {
    pub name: String,
    pub kind: String,
    pub last_polled_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_error: Option<String>,
    pub last_summary: Option<String>,
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

/// List every registered discovery source with its last-poll markers.
#[utoipa::path(
    get,
    path = "/api/v1/sources",
    tag = "sources",
    responses((status = 200, body = SourceList))
)]
pub async fn list(State(state): State<AppState>) -> ApiResult<Json<SourceList>> {
    let mut items = Vec::with_capacity(state.sources.len());
    for (name, source) in state.sources.iter() {
        let row = sources_repo::get(&state.db, source.kind(), name)
            .await
            .map_err(ApiError::Internal)?;
        items.push(SourceDto {
            name: name.to_string(),
            kind: source.kind().to_string(),
            last_polled_at: row.as_ref().and_then(|r| r.last_polled_at),
            last_success_at: row.as_ref().and_then(|r| r.last_success_at),
            last_error: row.as_ref().and_then(|r| r.last_error.clone()),
            last_summary: row.as_ref().and_then(|r| r.last_summary.clone()),
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

    let db = state.db.clone();
    let metadata = state.metadata.clone();
    let ingestion = state.ingestion.clone();
    let locks = state.locks.clone();
    tokio::spawn(async move {
        poll_source::run_tick(source, db, metadata, ingestion, locks).await;
    });

    Ok(Json(ManualPollResponse {
        source: name,
        triggered: true,
        skipped: false,
    }))
}
