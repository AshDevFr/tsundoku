//! Send-to-torrent-client integration: push a discovered release into the
//! operator's torrent client, plus an enablement/status probe. Both endpoints
//! are admin-gated (mounted under the `require_admin` layer in
//! [`crate::router`]).

use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use td_db::entities::{codex_health_checks, download_health_checks};
use td_db::repos::{TRIGGER_MANUAL, download_sends_repo, download_status_repo, releases_repo};
use td_download::{AddRequest, AddSource, DownloadError};
use utoipa::ToSchema;

use crate::errors::{ApiError, ApiResult};
use crate::handlers::releases::{ReleaseDto, model_to_release};
use crate::state::AppState;

/// Per-send overrides. Every field is optional; an empty body (`{}` or none)
/// sends with the configured defaults, which is the one-click path.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendToClientRequest {
    /// Label to apply for this send. Falls back to `download.default_label`.
    pub label: Option<String>,
    /// Whether to start the torrent immediately. Falls back to
    /// `download.default_start`.
    pub start: Option<bool>,
    /// Send a magnet URL instead of uploading the `.torrent` file for this
    /// send. Falls back to the inverse of `download.prefer_torrent_file`.
    pub prefer_magnet: Option<bool>,
}

/// One reachability-history entry. `trigger` is `launch` | `cron` | `manual`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckDto {
    /// Row id, unique within the table — a stable React key for the UI list.
    pub id: i64,
    pub checked_at: i64,
    pub reachable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub trigger: String,
}

impl From<download_health_checks::Model> for HealthCheckDto {
    fn from(m: download_health_checks::Model) -> Self {
        Self {
            id: m.id,
            checked_at: m.checked_at,
            reachable: m.reachable,
            error: m.error,
            trigger: m.trigger,
        }
    }
}

// Codex reuses the same DTO for its reachability history (identical shape).
impl From<codex_health_checks::Model> for HealthCheckDto {
    fn from(m: codex_health_checks::Model) -> Self {
        Self {
            id: m.id,
            checked_at: m.checked_at,
            reachable: m.reachable,
            error: m.error,
            trigger: m.trigger,
        }
    }
}

/// One send-attempt audit entry. `source` is `torrent` | `magnet`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendRecordDto {
    /// Row id, unique within the table — a stable React key for the UI list.
    pub id: i64,
    pub release_id: String,
    /// The release's title, so the log names what was sent instead of an
    /// opaque id. `None` only if the release row was removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_title: Option<String>,
    /// The resolved series id, so the UI can link the row to the series page.
    /// `None` when the release is unresolved (or the release row was removed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_id: Option<i32>,
    pub sent_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub source: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<download_sends_repo::SendWithTitle> for SendRecordDto {
    fn from(row: download_sends_repo::SendWithTitle) -> Self {
        let download_sends_repo::SendWithTitle {
            send: m,
            release_title,
            series_id,
        } = row;
        Self {
            id: m.id,
            release_id: m.release_id,
            release_title,
            series_id,
            sent_at: m.sent_at,
            label: m.label,
            source: m.source,
            success: m.success,
            error: m.error,
        }
    }
}

/// Connection info + live health for the admin Download page. Connection
/// fields come from the `[download]` config (the password is never exposed);
/// the health fields and history come from `download_status` /
/// `download_health_checks` / `download_sends`. When disabled, only `enabled`
/// is meaningful and the page renders a "configure me" notice.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatusDto {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Whether an HTTP Basic username is configured (the credential itself is
    /// never sent to the client).
    pub has_credentials: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_label: Option<String>,
    pub default_start: bool,
    pub prefer_torrent_file: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_cron: Option<String>,
    /// Last probe result. `false` until the first probe records one.
    pub reachable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_test_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_change_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub recent_checks: Vec<HealthCheckDto>,
    pub recent_sends: Vec<SendRecordDto>,
}

/// How many history rows to surface on the admin page.
const HISTORY_LIMIT: u64 = 20;

/// Assemble the status DTO from the config snapshot plus the persisted health /
/// audit rows. Shared by `status` (read) and `test` (read-after-probe).
async fn build_status_dto(state: &AppState) -> ApiResult<DownloadStatusDto> {
    let cfg = &state.download;
    let snapshot = download_status_repo::get(&state.db)
        .await
        .map_err(ApiError::Internal)?;
    let recent_checks = download_status_repo::list_recent_checks(&state.db, HISTORY_LIMIT)
        .await
        .map_err(ApiError::Internal)?;
    let recent_sends = download_sends_repo::list_recent(&state.db, HISTORY_LIMIT)
        .await
        .map_err(ApiError::Internal)?;

    Ok(DownloadStatusDto {
        enabled: cfg.enabled,
        kind: cfg.enabled.then(|| cfg.kind.clone()),
        base_url: cfg.rutorrent.normalized_base_url(),
        has_credentials: cfg.rutorrent.username.is_some(),
        default_label: cfg.default_label.clone(),
        default_start: cfg.default_start,
        prefer_torrent_file: cfg.prefer_torrent_file,
        health_cron: cfg.health_cron.clone(),
        reachable: snapshot.as_ref().map(|s| s.reachable).unwrap_or(false),
        last_test_at: snapshot.as_ref().and_then(|s| s.last_test_at),
        last_change_at: snapshot.as_ref().and_then(|s| s.last_change_at),
        last_error: snapshot.and_then(|s| s.last_error),
        recent_checks: recent_checks.into_iter().map(Into::into).collect(),
        recent_sends: recent_sends.into_iter().map(Into::into).collect(),
    })
}

/// Push a discovered release into the configured torrent client. Returns the
/// updated [`ReleaseDto`] (with the `sentToClientAt` badge fields set) so the
/// frontend can update the card from the response without a refetch.
#[utoipa::path(
    post,
    path = "/api/v1/releases/{id}/send-to-client",
    tag = "download",
    operation_id = "send_to_client",
    params(("id" = String, Path, description = "release id")),
    request_body = SendToClientRequest,
    responses(
        (status = 200, body = ReleaseDto),
        (status = 400, description = "Release has neither a magnet nor a torrent URL"),
        (status = 404, description = "No release with that id"),
        (status = 502, description = "The torrent client rejected the add or was unreachable"),
        (status = 503, description = "Download integration is disabled")
    ),
    security(("admin" = []))
)]
pub async fn send(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<SendToClientRequest>>,
) -> ApiResult<Json<ReleaseDto>> {
    let req = body.map(|Json(r)| r).unwrap_or_default();

    // Disabled integration short-circuits before any DB work, mirroring the
    // codex refresh handler.
    let client = state
        .download_client
        .clone()
        .ok_or_else(|| ApiError::Misconfigured("download integration is disabled".into()))?;

    let release = releases_repo::find_by_id(&state.db, &id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::NotFound(format!("release {id}")))?;

    // Resolve the source. The config default (`prefer_torrent_file`) is
    // overridable per send via `preferMagnet`; if the preferred kind is not
    // available on the release we fall back to the other rather than failing.
    let want_magnet = req
        .prefer_magnet
        .unwrap_or(!state.download.prefer_torrent_file);
    let source = resolve_source(&*client, &release, want_magnet).await?;
    let source_kind = match &source {
        AddSource::Torrent { .. } => "torrent",
        AddSource::Magnet(_) => "magnet",
    };

    let label = req
        .label
        .clone()
        .or_else(|| state.download.default_label.clone());
    let start = req.start.unwrap_or(state.download.default_start);
    let dir = state.download.default_dir.clone();

    let now = chrono::Utc::now().timestamp();
    // Record every attempt, success or failure, so a bounced send leaves a
    // trace instead of only a transient 502.
    match client
        .add(AddRequest {
            source,
            label: label.clone(),
            start,
            dir,
        })
        .await
    {
        Ok(()) => {
            download_sends_repo::insert(
                &state.db,
                &release.id,
                now,
                label.as_deref(),
                source_kind,
                true,
                None,
            )
            .await
            .map_err(ApiError::Internal)?;
            releases_repo::mark_sent_to_client(&state.db, &release.id, now, label)
                .await
                .map_err(ApiError::Internal)?;
        }
        Err(e) => {
            let message = e.to_string();
            download_sends_repo::insert(
                &state.db,
                &release.id,
                now,
                label.as_deref(),
                source_kind,
                false,
                Some(&message),
            )
            .await
            .map_err(ApiError::Internal)?;
            return Err(map_download_err(e));
        }
    }

    let row = releases_repo::find_by_id(&state.db, &release.id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::NotFound(format!("release {id}")))?;
    let formats = releases_repo::list_formats(&state.db, &row.id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(model_to_release(row, formats)))
}

/// Connection info + health snapshot for the admin UI; gates rendering of the
/// send button and powers the Download page.
#[utoipa::path(
    get,
    path = "/api/v1/download/status",
    tag = "download",
    operation_id = "download_status",
    responses((status = 200, body = DownloadStatusDto)),
    security(("admin" = []))
)]
pub async fn status(State(state): State<AppState>) -> ApiResult<Json<DownloadStatusDto>> {
    Ok(Json(build_status_dto(&state).await?))
}

/// Run an on-demand connection test and return the refreshed status. A failed
/// probe is **not** an error: it returns `200` with `reachable: false` and the
/// reason in `lastError` (a successful *report* of an unreachable client),
/// distinct from `503` when the integration is disabled. The manual test always
/// appends a history row.
#[utoipa::path(
    post,
    path = "/api/v1/download/test",
    tag = "download",
    operation_id = "download_test",
    responses(
        (status = 200, body = DownloadStatusDto),
        (status = 503, description = "Download integration is disabled")
    ),
    security(("admin" = []))
)]
pub async fn test(State(state): State<AppState>) -> ApiResult<Json<DownloadStatusDto>> {
    let client = state
        .download_client
        .clone()
        .ok_or_else(|| ApiError::Misconfigured("download integration is disabled".into()))?;

    let now = chrono::Utc::now().timestamp();
    let (reachable, error) = match client.test_connection().await {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };
    download_status_repo::record_check(&state.db, reachable, error.as_deref(), now, TRIGGER_MANUAL)
        .await
        .map_err(ApiError::Internal)?;

    Ok(Json(build_status_dto(&state).await?))
}

/// Pick the `AddSource` for a release, fetching the `.torrent` bytes through
/// the client's limiter when the torrent-file path wins. `want_magnet` decides
/// the preference; the non-preferred kind is used as a fallback, and a release
/// with neither source is a 400.
async fn resolve_source(
    client: &dyn td_download::DownloadClient,
    release: &td_db::repos::releases_repo::Model,
    want_magnet: bool,
) -> ApiResult<AddSource> {
    let magnet = release.magnet.as_deref();
    let torrent_url = release.torrent_url.as_deref();

    // Two preference orders; the non-preferred kind is the fallback.
    let order: [Source<'_>; 2] = if want_magnet {
        [Source::Magnet(magnet), Source::Torrent(torrent_url)]
    } else {
        [Source::Torrent(torrent_url), Source::Magnet(magnet)]
    };

    for candidate in order {
        match candidate {
            Source::Magnet(Some(m)) => return Ok(AddSource::Magnet(m.to_string())),
            Source::Torrent(Some(t)) => {
                return fetch_torrent(client, &release.id, t).await;
            }
            _ => continue,
        }
    }

    Err(ApiError::BadRequest(
        "release has neither a magnet nor a torrent url to send".into(),
    ))
}

/// A candidate source kind paired with its (possibly absent) URL, used to
/// express the preference order without duplicating the branch bodies.
enum Source<'a> {
    Magnet(Option<&'a str>),
    Torrent(Option<&'a str>),
}

/// Fetch the `.torrent` bytes and wrap them in an `AddSource::Torrent` with a
/// best-effort file name.
async fn fetch_torrent(
    client: &dyn td_download::DownloadClient,
    release_id: &str,
    url: &str,
) -> ApiResult<AddSource> {
    let bytes = client.fetch_torrent(url).await.map_err(map_download_err)?;
    Ok(AddSource::Torrent {
        bytes,
        file_name: torrent_file_name(release_id, url),
    })
}

/// Derive a `.torrent` file name for the upload: the URL's last path segment
/// when it already looks like a torrent file, else `<release_id>.torrent`.
/// ruTorrent mostly ignores the name, but a sensible one keeps client UIs tidy.
fn torrent_file_name(release_id: &str, url: &str) -> String {
    url.rsplit('/')
        .next()
        .map(|s| s.split(['?', '#']).next().unwrap_or(s))
        .filter(|s| s.ends_with(".torrent") && s.len() > ".torrent".len())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{release_id}.torrent"))
}

/// Map a download-client failure to a `502 Bad Gateway` carrying the client's
/// own error string, so the operator sees *why* the send failed (a rejected
/// add, an auth failure, an unreachable host) rather than a generic 500.
fn map_download_err(e: DownloadError) -> ApiError {
    ApiError::BadGateway(e.to_string())
}
