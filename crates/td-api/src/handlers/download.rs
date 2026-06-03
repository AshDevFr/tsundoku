//! Send-to-torrent-client integration: push a discovered release into the
//! operator's torrent client, plus an enablement/status probe. Both endpoints
//! are admin-gated (mounted under the `require_admin` layer in
//! [`crate::router`]).

use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use td_db::repos::releases_repo;
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

/// Enablement snapshot for the admin UI. When disabled, `kind` is omitted.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatusDto {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
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

    let label = req
        .label
        .clone()
        .or_else(|| state.download.default_label.clone());
    let start = req.start.unwrap_or(state.download.default_start);
    let dir = state.download.default_dir.clone();

    client
        .add(AddRequest {
            source,
            label: label.clone(),
            start,
            dir,
        })
        .await
        .map_err(map_download_err)?;

    let now = chrono::Utc::now().timestamp();
    releases_repo::mark_sent_to_client(&state.db, &release.id, now, label)
        .await
        .map_err(ApiError::Internal)?;

    let row = releases_repo::find_by_id(&state.db, &release.id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::NotFound(format!("release {id}")))?;
    let formats = releases_repo::list_formats(&state.db, &row.id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(model_to_release(row, formats)))
}

/// Enablement probe for the admin UI; gates rendering of the send button.
#[utoipa::path(
    get,
    path = "/api/v1/download/status",
    tag = "download",
    operation_id = "download_status",
    responses((status = 200, body = DownloadStatusDto)),
    security(("admin" = []))
)]
pub async fn status(State(state): State<AppState>) -> Json<DownloadStatusDto> {
    if state.download.enabled {
        Json(DownloadStatusDto {
            enabled: true,
            kind: Some(state.download.kind.clone()),
        })
    } else {
        Json(DownloadStatusDto {
            enabled: false,
            kind: None,
        })
    }
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
