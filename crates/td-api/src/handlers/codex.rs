//! Codex presence integration: manual refresh trigger, connection-health
//! status, and manual series linking. All admin-gated (mounted under the
//! `require_admin` layer in [`crate::router`]).

use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use td_db::repos::{TRIGGER_MANUAL, codex_link_repo, codex_status_repo, series_repo};
use td_scheduler::dispatch;
use td_scheduler::jobs::sync_codex;
use utoipa::ToSchema;

use crate::errors::{ApiError, ApiResult};
use crate::handlers::download::HealthCheckDto;
use crate::state::{AppState, JobKind, JobResult};

/// How many reachability-history rows to surface on the admin Codex card.
const HISTORY_LIMIT: u64 = 20;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexRefreshResponse {
    pub triggered: bool,
    /// `false` when a previous sweep is still in flight; the request is a no-op.
    pub skipped: bool,
}

/// Connection-health snapshot for the admin UI. When the integration is
/// disabled, only `enabled: false` is meaningful; the rest are `None`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexStatusDto {
    pub enabled: bool,
    pub reachable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_version: Option<String>,
    /// `unknown` | `ok` | `unauthorized` (401) | `forbidden` (403).
    pub auth_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_preflight_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Series matched to a tsundoku series by the last successful sweep.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linked_count: Option<i64>,
    /// Series pulled from Codex by the last successful sweep (superset of
    /// `linked_count`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched_count: Option<i64>,
    /// Recent reachability transitions + manual tests, newest first. Empty when
    /// disabled or before the first probe.
    pub recent_checks: Vec<HealthCheckDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexLinkRequest {
    /// Codex series UUID to hand-link this tsundoku series to.
    pub codex_series_uuid: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexLinkResponse {
    pub series_id: i32,
    pub codex_series_uuid: String,
    /// Always `manual` for a hand-link.
    pub link_kind: String,
}

/// Trigger a Codex presence sweep. Shares the single codex lock with the cron,
/// so a manual kick during a scheduled sweep is reported `skipped`.
#[utoipa::path(
    post,
    path = "/api/v1/codex/refresh",
    tag = "codex",
    operation_id = "codex_refresh",
    responses(
        (status = 202, body = CodexRefreshResponse),
        (status = 503, description = "Codex integration is disabled")
    ),
    security(("admin" = []))
)]
pub async fn refresh(State(state): State<AppState>) -> ApiResult<Json<CodexRefreshResponse>> {
    let client = state
        .codex_client
        .clone()
        .ok_or_else(|| ApiError::Misconfigured("codex integration is disabled".into()))?;

    let lock = state.locks.codex_sync_lock();
    let db = state.db.clone();
    let triggered = dispatch::try_dispatch(
        &state.job_events,
        lock,
        JobKind::Codex,
        sync_codex::JOB_KEY,
        || async {},
        move || async move {
            sync_codex::run_tick(client, db).await;
            JobResult {
                triggered: true,
                skipped: false,
                ..Default::default()
            }
        },
    );

    Ok(Json(CodexRefreshResponse {
        triggered,
        skipped: !triggered,
    }))
}

/// Assemble the status DTO from the persisted snapshot + reachability history.
/// Shared by `status` (read) and `test` (read-after-probe). Returns the bare
/// DTO; callers wrap it in `Json`.
async fn build_status_dto(state: &AppState) -> ApiResult<CodexStatusDto> {
    if !state.codex.enabled {
        return Ok(CodexStatusDto {
            enabled: false,
            reachable: false,
            codex_name: None,
            codex_version: None,
            auth_state: codex_status_repo::AUTH_UNKNOWN.to_string(),
            last_preflight_at: None,
            last_success_at: None,
            last_error: None,
            linked_count: None,
            fetched_count: None,
            recent_checks: Vec::new(),
        });
    }

    let row = codex_status_repo::get(&state.db)
        .await
        .map_err(ApiError::Internal)?;
    let recent_checks = codex_status_repo::list_recent_checks(&state.db, HISTORY_LIMIT)
        .await
        .map_err(ApiError::Internal)?
        .into_iter()
        .map(Into::into)
        .collect();

    let dto = match row {
        Some(r) => CodexStatusDto {
            enabled: true,
            reachable: r.reachable,
            codex_name: r.codex_name,
            codex_version: r.codex_version,
            auth_state: r.auth_state,
            last_preflight_at: r.last_preflight_at,
            last_success_at: r.last_success_at,
            last_error: r.last_error,
            linked_count: r.linked_count,
            fetched_count: r.fetched_count,
            recent_checks,
        },
        // Enabled but no sweep has run yet (e.g. fresh boot before the first
        // cron tick): report enabled + unknown rather than a misleading row.
        None => CodexStatusDto {
            enabled: true,
            reachable: false,
            codex_name: None,
            codex_version: None,
            auth_state: codex_status_repo::AUTH_UNKNOWN.to_string(),
            last_preflight_at: None,
            last_success_at: None,
            last_error: None,
            linked_count: None,
            fetched_count: None,
            recent_checks,
        },
    };
    Ok(dto)
}

/// Codex connection-health status for the admin UI.
#[utoipa::path(
    get,
    path = "/api/v1/codex/status",
    tag = "codex",
    operation_id = "codex_status",
    responses((status = 200, body = CodexStatusDto)),
    security(("admin" = []))
)]
pub async fn status(State(state): State<AppState>) -> ApiResult<Json<CodexStatusDto>> {
    Ok(Json(build_status_dto(&state).await?))
}

/// Run an on-demand Codex `/info` preflight and return the refreshed status.
/// Like the download test, a failed probe is **not** an error: it returns
/// `200` with `reachable: false` and records a `manual` history row, distinct
/// from `503` when the integration is disabled.
#[utoipa::path(
    post,
    path = "/api/v1/codex/test",
    tag = "codex",
    operation_id = "codex_test",
    responses(
        (status = 200, body = CodexStatusDto),
        (status = 503, description = "Codex integration is disabled")
    ),
    security(("admin" = []))
)]
pub async fn test(State(state): State<AppState>) -> ApiResult<Json<CodexStatusDto>> {
    let client = state
        .codex_client
        .clone()
        .ok_or_else(|| ApiError::Misconfigured("codex integration is disabled".into()))?;

    let now = chrono::Utc::now().timestamp();
    let outcome = match client.info().await {
        Ok(info) => {
            codex_status_repo::record_preflight(
                &state.db,
                true,
                Some(&info.name),
                Some(&info.version),
                None,
                now,
                TRIGGER_MANUAL,
            )
            .await
        }
        Err(e) => {
            codex_status_repo::record_preflight(
                &state.db,
                false,
                None,
                None,
                Some(&e.to_string()),
                now,
                TRIGGER_MANUAL,
            )
            .await
        }
    };
    outcome.map_err(ApiError::Internal)?;

    Ok(Json(build_status_dto(&state).await?))
}

/// Hand-link a tsundoku series to a Codex series UUID. For series with no
/// matchable external id; the next sweep refreshes the link's counts by uuid.
#[utoipa::path(
    post,
    path = "/api/v1/series/{id}/codex-link",
    tag = "codex",
    operation_id = "codex_link",
    params(("id" = i32, Path, description = "tsundoku series id")),
    request_body = CodexLinkRequest,
    responses(
        (status = 200, body = CodexLinkResponse),
        (status = 404, description = "No series with that id")
    ),
    security(("admin" = []))
)]
pub async fn link(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<CodexLinkRequest>,
) -> ApiResult<Json<CodexLinkResponse>> {
    let uuid = req.codex_series_uuid.trim();
    if uuid.is_empty() {
        return Err(ApiError::BadRequest(
            "codexSeriesUuid must not be empty".into(),
        ));
    }
    // Confirm the series exists so a bad id is a clean 404, not an FK error.
    if series_repo::find_by_id(&state.db, id)
        .await
        .map_err(ApiError::Internal)?
        .is_none()
    {
        return Err(ApiError::NotFound(format!("series {id}")));
    }

    let now = chrono::Utc::now().timestamp();
    codex_link_repo::upsert_manual(&state.db, id, uuid, now)
        .await
        .map_err(ApiError::Internal)?;

    Ok(Json(CodexLinkResponse {
        series_id: id,
        codex_series_uuid: uuid.to_string(),
        link_kind: codex_link_repo::KIND_MANUAL.to_string(),
    }))
}

/// Remove a series' Codex link (manual or auto). Idempotent: unlinking a
/// series with no link is a no-op success.
#[utoipa::path(
    delete,
    path = "/api/v1/series/{id}/codex-link",
    tag = "codex",
    operation_id = "codex_unlink",
    params(("id" = i32, Path, description = "tsundoku series id")),
    responses((status = 204, description = "Link removed (or none existed)")),
    security(("admin" = []))
)]
pub async fn unlink(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> ApiResult<axum::http::StatusCode> {
    codex_link_repo::delete(&state.db, id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
