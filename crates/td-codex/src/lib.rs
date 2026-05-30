//! Codex HTTP client for tsundoku's presence overlay.
//!
//! Two calls back the integration:
//!
//! - [`CodexClient::info`] hits Codex's **public** `GET /api/v1/info`
//!   (`{ version, name }`). It needs no credentials, so it doubles as a cheap
//!   reachability/version probe at startup and before each sweep. A 200 proves
//!   Codex is up and what version it is — it says nothing about whether the
//!   api_key is valid.
//! - [`CodexClient::fetch_external_index_page`] / [`CodexClient::fetch_all`]
//!   hit the authenticated slim `GET /api/v1/series/external-index`, sending
//!   the Codex `X-API-Key`. 401/403 are surfaced as distinct
//!   [`CodexError::Unauthorized`] / [`CodexError::Forbidden`] so the caller can
//!   record which operator fix is needed.
//!
//! The client carries no `td-config` dependency: callers pass the resolved
//! base URL, key, and timeout so the crate stays usable from both `td-api` and
//! `td-scheduler`.

use std::sync::Arc;
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use td_http::{HttpLimiter, LimitedClient};

/// Errors from a Codex call. `Unauthorized` (401) and `Forbidden` (403) are
/// split out because they map to different operator fixes: a wrong/missing
/// api_key vs a key that authenticates but lacks the `series:read` scope.
#[derive(Debug, thiserror::Error)]
pub enum CodexError {
    #[error("codex request transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("codex rejected the api_key (401 Unauthorized)")]
    Unauthorized,
    #[error("codex api_key lacks the series:read scope (403 Forbidden)")]
    Forbidden,
    #[error("codex returned unexpected HTTP {0}")]
    Unexpected(u16),
    #[error("decoding codex response from {url}: {source}")]
    Decode {
        url: String,
        #[source]
        source: serde_json::Error,
    },
}

/// `GET /api/v1/info` body. Codex marks this endpoint public.
#[derive(Debug, Clone, Deserialize)]
pub struct CodexInfo {
    pub version: String,
    pub name: String,
}

/// One page of `GET /api/v1/series/external-index` (Codex's `PaginatedResponse`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIndexPage {
    pub items: Vec<ExternalIndexItem>,
    pub page: u64,
    pub page_size: u64,
    pub total: u64,
}

/// One Codex series projected to just what the overlay needs.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIndexItem {
    /// Codex series UUID (used for the deep link).
    pub id: String,
    /// External IDs linked on Codex; may be empty.
    #[serde(default)]
    pub external_ids: Vec<ExternalIdRef>,
    /// Highest owned volume number (Codex `MAX(volume)`); `None` if none
    /// parsed. The comparison basis for the presence status.
    pub local_max_volume: Option<f64>,
    /// Highest owned chapter number (Codex `MAX(chapter)`); `None` if none
    /// parsed.
    pub local_max_chapter: Option<f64>,
    /// Count of complete-volume files. Soft, display-only — never compared.
    pub volumes_owned: Option<i64>,
}

/// A single external-id reference on a Codex series.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIdRef {
    /// Codex source string, e.g. `plugin:mangabaka`, `api:mangabaka`, `manual`.
    pub source: String,
    pub external_id: String,
    #[serde(default)]
    pub external_url: Option<String>,
}

/// Map a Codex external-id `source` string to the tsundoku provider name used
/// in `series_external_ids`, or `None` when there is no tsundoku equivalent.
///
/// Codex namespaces its sources (`plugin:mangabaka`, `api:anilist`, …) and also
/// records non-provider origins (`comicinfo`, `epub`, `manual`). tsundoku stores
/// bare provider names: `mangabaka`, `mangaupdates`, `anilist`, `mal`. So we
/// strip a leading `plugin:` / `api:`, lowercase, and alias `myanimelist` /
/// `my_anime_list` to `mal`. Sources outside that set return `None` and simply
/// don't participate in matching.
pub fn normalize_source(codex_source: &str) -> Option<String> {
    let s = codex_source.trim().to_ascii_lowercase();
    let bare = s
        .strip_prefix("plugin:")
        .or_else(|| s.strip_prefix("api:"))
        .unwrap_or(&s)
        .trim();
    let provider = match bare {
        "mangabaka" => "mangabaka",
        "mangaupdates" | "manga_updates" => "mangaupdates",
        "anilist" => "anilist",
        "mal" | "myanimelist" | "my_anime_list" => "mal",
        // comicinfo / epub / manual / unknown: no tsundoku provider mapping.
        _ => return None,
    };
    Some(provider.to_string())
}

/// Page size used when sweeping the whole external index. Codex caps page size
/// server-side; this stays under common caps while keeping the sweep to few
/// round trips at personal scale.
pub const DEFAULT_SWEEP_PAGE_SIZE: u64 = 100;

/// Thin Codex API client. Construct once and reuse; the underlying
/// [`LimitedClient`] shares the process-wide host limiter.
pub struct CodexClient {
    http: LimitedClient,
    base_url: String,
    api_key: String,
}

impl CodexClient {
    /// `base_url` should already be normalized (no trailing slash); see
    /// `td_config::CodexConfig::normalized_base_url`.
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        timeout: Duration,
        limiter: Arc<HttpLimiter>,
    ) -> Result<Self, CodexError> {
        let inner = Client::builder()
            .timeout(timeout)
            .user_agent(concat!("tsundoku/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http: limiter.client(inner),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
        })
    }

    /// Public reachability/version probe. No api_key is sent (the endpoint is
    /// public), so a success here does not validate credentials.
    pub async fn info(&self) -> Result<CodexInfo, CodexError> {
        let url = format!("{}/api/v1/info", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header("accept", "application/json")
            .send()
            .await?;
        match resp.status() {
            StatusCode::OK => decode(&url, resp.bytes().await?.as_ref()),
            other => Err(CodexError::Unexpected(other.as_u16())),
        }
    }

    /// One page of the external index. Sends the api_key. Maps 401/403 to the
    /// dedicated error variants.
    pub async fn fetch_external_index_page(
        &self,
        page: u64,
        page_size: u64,
    ) -> Result<ExternalIndexPage, CodexError> {
        let url = format!(
            "{}/api/v1/series/external-index?page={}&pageSize={}",
            self.base_url, page, page_size
        );
        let resp = self
            .http
            .get(&url)
            .header("accept", "application/json")
            .header("x-api-key", &self.api_key)
            .send()
            .await?;
        match resp.status() {
            StatusCode::OK => decode(&url, resp.bytes().await?.as_ref()),
            StatusCode::UNAUTHORIZED => Err(CodexError::Unauthorized),
            StatusCode::FORBIDDEN => Err(CodexError::Forbidden),
            other => Err(CodexError::Unexpected(other.as_u16())),
        }
    }

    /// Sweep every page of the external index into one vec. Stops once the
    /// collected count reaches the server-reported `total` (or a page comes
    /// back empty, guarding against a drifting total mid-sweep).
    pub async fn fetch_all(&self, page_size: u64) -> Result<Vec<ExternalIndexItem>, CodexError> {
        let page_size = page_size.max(1);
        let mut out: Vec<ExternalIndexItem> = Vec::new();
        let mut page = 1u64;
        loop {
            let p = self.fetch_external_index_page(page, page_size).await?;
            let got = p.items.len();
            let total = p.total;
            out.extend(p.items);
            if got == 0 || !needs_more_pages(out.len() as u64, total) {
                break;
            }
            page += 1;
        }
        Ok(out)
    }
}

/// Whether a sweep should request another page: more rows are claimed by
/// `total` than we've collected. Pure so the pagination decision is testable
/// without a live server.
fn needs_more_pages(collected: u64, total: u64) -> bool {
    collected < total
}

fn decode<T: for<'de> Deserialize<'de>>(url: &str, bytes: &[u8]) -> Result<T, CodexError> {
    serde_json::from_slice(bytes).map_err(|source| CodexError::Decode {
        url: url.to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = include_str!("../tests/fixtures/info_response.json");
    const PAGE: &str = include_str!("../tests/fixtures/external_index_page.json");

    #[test]
    fn deserializes_info() {
        let info: CodexInfo = serde_json::from_str(INFO).unwrap();
        assert_eq!(info.name, "codex");
        assert_eq!(info.version, "1.4.2");
    }

    #[test]
    fn deserializes_external_index_page() {
        let page: ExternalIndexPage = serde_json::from_str(PAGE).unwrap();
        assert_eq!(page.page, 1);
        assert_eq!(page.page_size, 100);
        assert_eq!(page.total, 2);
        assert_eq!(page.items.len(), 2);

        // First item: linked + counts present (integer volume decodes to f64).
        let first = &page.items[0];
        assert_eq!(first.id, "550e8400-e29b-41d4-a716-446655440002");
        assert_eq!(first.external_ids.len(), 1);
        assert_eq!(first.external_ids[0].source, "plugin:mangabaka");
        assert_eq!(first.external_ids[0].external_id, "12345");
        assert_eq!(
            first.external_ids[0].external_url.as_deref(),
            Some("https://mangabaka.dev/manga/12345")
        );
        assert_eq!(first.local_max_volume, Some(12.0));
        assert_eq!(first.local_max_chapter, Some(130.5));
        assert_eq!(first.volumes_owned, Some(12));

        // Second item: no external ids, no parsed maxima — the manual-link case.
        let second = &page.items[1];
        assert!(second.external_ids.is_empty());
        assert!(second.local_max_volume.is_none());
        assert!(second.local_max_chapter.is_none());
    }

    #[test]
    fn normalize_source_strips_prefixes_and_aliases() {
        // plugin: / api: prefixes are stripped.
        assert_eq!(
            normalize_source("plugin:mangabaka").as_deref(),
            Some("mangabaka")
        );
        assert_eq!(
            normalize_source("api:mangabaka").as_deref(),
            Some("mangabaka")
        );
        assert_eq!(normalize_source("mangabaka").as_deref(), Some("mangabaka"));
        // myanimelist variants alias to tsundoku's `mal`.
        assert_eq!(normalize_source("api:myanimelist").as_deref(), Some("mal"));
        assert_eq!(
            normalize_source("plugin:my_anime_list").as_deref(),
            Some("mal")
        );
        assert_eq!(normalize_source("mal").as_deref(), Some("mal"));
        // anilist + mangaupdates pass through.
        assert_eq!(normalize_source("api:anilist").as_deref(), Some("anilist"));
        assert_eq!(
            normalize_source("plugin:mangaupdates").as_deref(),
            Some("mangaupdates")
        );
        // Case-insensitive.
        assert_eq!(
            normalize_source("Plugin:MangaBaka").as_deref(),
            Some("mangabaka")
        );
    }

    #[test]
    fn normalize_source_rejects_non_provider_origins() {
        assert!(normalize_source("comicinfo").is_none());
        assert!(normalize_source("epub").is_none());
        assert!(normalize_source("manual").is_none());
        assert!(normalize_source("plugin:somethingelse").is_none());
        assert!(normalize_source("").is_none());
    }

    #[test]
    fn pagination_stops_when_total_reached() {
        assert!(needs_more_pages(0, 2));
        assert!(needs_more_pages(100, 250));
        assert!(!needs_more_pages(2, 2));
        assert!(!needs_more_pages(250, 250));
        // A 0-total empty library never asks for more.
        assert!(!needs_more_pages(0, 0));
    }
}
