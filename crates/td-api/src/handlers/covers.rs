//! Cover-image proxy + on-disk cache.
//!
//! Three endpoints:
//! - `GET /api/v1/covers/{series_id}` resolves the cover URL stored on
//!   the series row, fetches once, then serves every subsequent hit from
//!   the on-disk cache.
//! - `GET /api/v1/covers/by-url?url=<absolute URL>` does the same for an
//!   arbitrary URL after a hardcoded host allowlist check. Used by the
//!   review/search UI where the candidate series isn't persisted yet.
//! - `POST /api/v1/covers/invalidate-cache` (admin) wipes every file
//!   under `cover_cache_dir` and reports counts.
//!
//! Cache layout is content-addressed: filename is
//! `sha256(url).<ext>`. When a cover URL rotates upstream, the new hash
//! triggers a fresh fetch; the stale file is harmless until the next
//! invalidate.

use std::path::Path as StdPath;
use std::sync::OnceLock;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Json, Response};
use sea_orm::EntityTrait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use td_db::entities::series;
use utoipa::{IntoParams, ToSchema};

use crate::errors::{ApiError, ApiResult};
use crate::state::AppState;

/// Cache-Control sent with every cover response. Short enough that
/// hitting "Invalidate cover cache" in the admin UI feels effective on
/// the next page navigation, long enough that scrolling a list does not
/// re-validate on every card.
const CACHE_CONTROL: &str = "public, max-age=3600";

/// Hardcoded host allowlist for the `by-url` endpoint. Anything that is
/// not exactly one of these or a subdomain is rejected with 400. Kept
/// minimal because the only documented use case is the review/search UI
/// surfacing MangaBaka search hits.
const ALLOWED_HOSTS: &[&str] = &["mangabaka.dev"];

/// Lazy outbound client. One pool process-wide. A short timeout keeps a
/// hung upstream from hogging a connection slot.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!(
                "tsundoku/",
                env!("CARGO_PKG_VERSION"),
                " (cover-proxy)"
            ))
            .timeout(Duration::from_secs(10))
            .build()
            .expect("default reqwest client builds")
    })
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct ByUrlQuery {
    /// Absolute HTTPS URL of the cover image. Must be on the allowlist.
    pub url: String,
}

/// Response payload for `POST /api/v1/covers/invalidate-cache`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InvalidateCoverCacheResponse {
    /// Number of files deleted from `cover_cache_dir`. Subdirectories
    /// and hidden files are ignored.
    pub files_deleted: u32,
    /// Total bytes freed across the deleted files.
    pub bytes_freed: u64,
}

/// Serve the cached cover for a series, fetching upstream once on miss.
#[utoipa::path(
    get,
    path = "/api/v1/covers/{series_id}",
    tag = "covers",
    operation_id = "get_cover_by_series_id",
    params(("series_id" = i32, Path, description = "Internal series id")),
    responses(
        (status = 200, description = "Cover image bytes", content_type = "image/*"),
        (status = 404, description = "Series missing or has no cover URL"),
        (status = 502, description = "Upstream fetch failed"),
        (status = 503, description = "Cover cache directory not configured"),
    ),
)]
pub async fn get_by_series_id(
    State(state): State<AppState>,
    Path(series_id): Path<i32>,
) -> ApiResult<Response> {
    let dir = state
        .cover_cache_dir
        .as_deref()
        .ok_or_else(|| ApiError::Misconfigured("cover cache directory not configured".into()))?;
    let row = series::Entity::find_by_id(series_id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .ok_or_else(|| ApiError::NotFound(format!("series {series_id} not found")))?;
    let url = row
        .cover_url
        .ok_or_else(|| ApiError::NotFound(format!("series {series_id} has no cover URL")))?;
    serve_or_fetch(dir, &url).await
}

/// Proxy + cache an arbitrary cover URL after host-allowlist validation.
#[utoipa::path(
    get,
    path = "/api/v1/covers/by-url",
    tag = "covers",
    operation_id = "get_cover_by_url",
    params(ByUrlQuery),
    responses(
        (status = 200, description = "Cover image bytes", content_type = "image/*"),
        (status = 400, description = "Missing, invalid, or disallowed URL"),
        (status = 502, description = "Upstream fetch failed"),
        (status = 503, description = "Cover cache directory not configured"),
    ),
)]
pub async fn get_by_url(
    State(state): State<AppState>,
    Query(q): Query<ByUrlQuery>,
) -> ApiResult<Response> {
    let dir = state
        .cover_cache_dir
        .as_deref()
        .ok_or_else(|| ApiError::Misconfigured("cover cache directory not configured".into()))?;
    validate_allowed_url(&q.url)?;
    serve_or_fetch(dir, &q.url).await
}

/// Wipe every file directly under `cover_cache_dir`. Subdirectories are
/// preserved so an operator who keeps a sibling directory for unrelated
/// artifacts does not lose it.
#[utoipa::path(
    post,
    path = "/api/v1/covers/invalidate-cache",
    tag = "covers",
    operation_id = "invalidate_cover_cache",
    responses(
        (status = 200, body = InvalidateCoverCacheResponse),
        (status = 503, description = "Cover cache directory not configured"),
    ),
    security(("admin" = []))
)]
pub async fn invalidate_cache(
    State(state): State<AppState>,
) -> ApiResult<Json<InvalidateCoverCacheResponse>> {
    let dir = state
        .cover_cache_dir
        .as_deref()
        .ok_or_else(|| ApiError::Misconfigured("cover cache directory not configured".into()))?;
    let (files_deleted, bytes_freed) = wipe_cache(dir).await.map_err(ApiError::Internal)?;
    Ok(Json(InvalidateCoverCacheResponse {
        files_deleted,
        bytes_freed,
    }))
}

/// Common path: look in the cache, fall through to upstream on miss.
async fn serve_or_fetch(dir: &StdPath, url: &str) -> ApiResult<Response> {
    let (filename, content_type) = cache_filename(url);
    let path = dir.join(&filename);
    if let Ok(bytes) = tokio::fs::read(&path).await {
        return Ok(image_response(bytes, content_type));
    }
    let bytes = fetch_upstream(url).await?;
    if let Err(e) = write_atomic(dir, &filename, &bytes).await {
        // A failed write is not fatal: the client still gets the bytes.
        // Re-fetch on the next request will retry the cache write.
        tracing::warn!(error = ?e, %url, "cover cache write failed; serving bytes uncached");
    }
    Ok(image_response(bytes, content_type))
}

/// Pull bytes from the upstream cover URL. Maps an upstream 404 to a
/// matching `NotFound` API error, anything else non-2xx to an internal
/// error (rendered as 500). Returning 502 here would more faithfully
/// reflect "upstream is broken," but our error model collapses the
/// distinction; a 500 is fine for a cover proxy.
async fn fetch_upstream(url: &str) -> ApiResult<Vec<u8>> {
    let resp = http_client().get(url).send().await.map_err(|e| {
        tracing::warn!(error = ?e, %url, "upstream cover request failed");
        ApiError::Internal(anyhow::anyhow!("upstream request failed"))
    })?;
    let status = resp.status();
    if status == StatusCode::NOT_FOUND {
        return Err(ApiError::NotFound(format!("upstream 404 for {url}")));
    }
    if !status.is_success() {
        tracing::warn!(%url, %status, "upstream returned non-2xx for cover");
        return Err(ApiError::Internal(anyhow::anyhow!(
            "upstream returned {status} for cover"
        )));
    }
    let bytes = resp.bytes().await.map_err(|e| {
        tracing::warn!(error = ?e, %url, "reading upstream cover body failed");
        ApiError::Internal(anyhow::anyhow!("upstream body read failed"))
    })?;
    Ok(bytes.to_vec())
}

/// Build a `(filename, content_type)` pair from the URL. The filename is
/// `sha256(url).<ext>` and content type is sniffed from `<ext>`. URLs
/// without a recognized image extension fall back to `.jpg` /
/// `image/jpeg`, which matches every MangaBaka cover we've seen.
fn cache_filename(url: &str) -> (String, &'static str) {
    let hash = {
        let mut h = Sha256::new();
        h.update(url.as_bytes());
        hex_lower(&h.finalize())
    };
    let (ext, ct) = sniff_ext(url);
    (format!("{hash}.{ext}"), ct)
}

fn sniff_ext(url: &str) -> (&'static str, &'static str) {
    // Strip query / fragment before looking at the trailing path segment.
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".png") {
        ("png", "image/png")
    } else if lower.ends_with(".webp") {
        ("webp", "image/webp")
    } else if lower.ends_with(".gif") {
        ("gif", "image/gif")
    } else {
        ("jpg", "image/jpeg")
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Atomically write `bytes` to `dir/<filename>` by routing through a
/// temp file in the same directory and renaming. Concurrent writers
/// targeting the same hash race harmlessly: both produce identical
/// bytes and the last rename wins.
async fn write_atomic(dir: &StdPath, filename: &str, bytes: &[u8]) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let final_path = dir.join(filename);
    let tmp = dir.join(format!(
        "{filename}.tmp.{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, &final_path).await?;
    Ok(())
}

fn image_response(bytes: impl Into<Body>, content_type: &'static str) -> Response {
    let mut resp = Response::new(bytes.into());
    let headers = resp.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CACHE_CONTROL),
    );
    resp
}

/// Delete every regular file directly under `dir`. Directories and
/// hidden entries (dotfiles) are left alone. Failure to delete an
/// individual file is logged and skipped; the operation reports the
/// counts for the files that did go through.
async fn wipe_cache(dir: &StdPath) -> anyhow::Result<(u32, u64)> {
    if !dir.exists() {
        return Ok((0, 0));
    }
    let mut entries = tokio::fs::read_dir(dir).await?;
    let mut files = 0u32;
    let mut bytes = 0u64;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        let meta = match entry.metadata().await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = ?e, path = ?entry.path(), "stat failed during cover cache wipe");
                continue;
            }
        };
        if !meta.is_file() {
            continue;
        }
        let size = meta.len();
        match tokio::fs::remove_file(entry.path()).await {
            Ok(()) => {
                files += 1;
                bytes = bytes.saturating_add(size);
            }
            Err(e) => {
                tracing::warn!(error = ?e, path = ?entry.path(), "delete failed during cover cache wipe");
            }
        }
    }
    Ok((files, bytes))
}

/// Reject anything that is not HTTPS or whose host isn't on the
/// allowlist. Returns a 400 with a short reason; the URL is included in
/// the log line but not the response body (the caller already knows
/// what they sent).
fn validate_allowed_url(url: &str) -> ApiResult<()> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| ApiError::BadRequest("url is not a valid absolute URL".into()))?;
    if parsed.scheme() != "https" {
        return Err(ApiError::BadRequest("url must be https".into()));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| ApiError::BadRequest("url is missing a host".into()))?;
    let host_lc = host.to_ascii_lowercase();
    if !ALLOWED_HOSTS
        .iter()
        .any(|h| host_lc == *h || host_lc.ends_with(&format!(".{h}")))
    {
        return Err(ApiError::BadRequest(format!(
            "host {host} is not on the cover-proxy allowlist"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_jpg_from_url_with_query_string() {
        assert_eq!(
            sniff_ext("https://cdn.example.dev/foo/350.jpg?v=42"),
            ("jpg", "image/jpeg")
        );
    }

    #[test]
    fn sniffs_png_webp_gif() {
        assert_eq!(sniff_ext("https://x.dev/y.PNG"), ("png", "image/png"));
        assert_eq!(sniff_ext("https://x.dev/y.webp"), ("webp", "image/webp"));
        assert_eq!(sniff_ext("https://x.dev/y.gif"), ("gif", "image/gif"));
    }

    #[test]
    fn defaults_to_jpeg_when_extension_is_unknown() {
        assert_eq!(sniff_ext("https://x.dev/cover"), ("jpg", "image/jpeg"));
    }

    #[test]
    fn cache_filename_is_stable_per_url() {
        let (a, _) = cache_filename("https://x.dev/y.jpg");
        let (b, _) = cache_filename("https://x.dev/y.jpg");
        assert_eq!(a, b);
        assert!(a.ends_with(".jpg"));
        assert_eq!(a.len(), 64 + ".jpg".len());
    }

    #[test]
    fn cache_filename_differs_per_url() {
        let (a, _) = cache_filename("https://x.dev/a.jpg");
        let (b, _) = cache_filename("https://x.dev/b.jpg");
        assert_ne!(a, b);
    }

    #[test]
    fn allowlist_accepts_apex_and_subdomains() {
        assert!(validate_allowed_url("https://mangabaka.dev/x.jpg").is_ok());
        assert!(validate_allowed_url("https://cdn.mangabaka.dev/x.jpg").is_ok());
        assert!(validate_allowed_url("https://a.b.mangabaka.dev/x.jpg").is_ok());
    }

    #[test]
    fn allowlist_rejects_http_other_hosts_and_lookalikes() {
        assert!(validate_allowed_url("http://mangabaka.dev/x.jpg").is_err());
        assert!(validate_allowed_url("https://evil.example.com/x.jpg").is_err());
        // Lookalike: not a subdomain of mangabaka.dev, just ends with the
        // same string. The dot-prefix check rejects it.
        assert!(validate_allowed_url("https://notmangabaka.dev/x.jpg").is_err());
    }

    #[test]
    fn allowlist_rejects_malformed_input() {
        assert!(validate_allowed_url("not a url").is_err());
        assert!(validate_allowed_url("file:///etc/passwd").is_err());
    }

    #[tokio::test]
    async fn wipe_cache_skips_subdirs_and_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.jpg"), b"hello")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("b.png"), b"hi")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join(".gitkeep"), b"")
            .await
            .unwrap();
        tokio::fs::create_dir(dir.path().join("subdir"))
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("subdir").join("c.jpg"), b"x")
            .await
            .unwrap();

        let (files, bytes) = wipe_cache(dir.path()).await.unwrap();
        assert_eq!(files, 2);
        assert_eq!(bytes, 7);
        assert!(!dir.path().join("a.jpg").exists());
        assert!(!dir.path().join("b.png").exists());
        assert!(dir.path().join(".gitkeep").exists());
        assert!(dir.path().join("subdir").join("c.jpg").exists());
    }

    #[tokio::test]
    async fn wipe_cache_returns_zero_on_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        let (files, bytes) = wipe_cache(&missing).await.unwrap();
        assert_eq!((files, bytes), (0, 0));
    }

    #[tokio::test]
    async fn write_atomic_creates_dir_and_final_file() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b");
        write_atomic(&nested, "x.jpg", b"hello").await.unwrap();
        let read = tokio::fs::read(nested.join("x.jpg")).await.unwrap();
        assert_eq!(read, b"hello");
    }
}
