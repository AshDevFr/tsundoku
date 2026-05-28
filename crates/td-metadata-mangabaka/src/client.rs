//! MangaBaka HTTP client.
//!
//! Internal to this crate: callers use the `MetadataProvider` trait. The
//! client only knows about MangaBaka's URL shape and JSON envelope; the
//! mapping to canonical [`td_metadata::SeriesMetadata`] happens in
//! [`crate::mapping`].

use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use td_http::{HttpLimiter, LimitedClient};
use td_metadata::MetadataError;

use crate::PROVIDER_ID;

/// MangaBaka's JSON envelope. `status` mirrors the HTTP status code.
#[derive(Debug, Deserialize)]
pub struct MbEnvelope<T> {
    #[allow(dead_code)]
    pub status: i32,
    pub data: T,
    #[serde(default)]
    pub pagination: Option<MbPagination>,
}

/// MangaBaka's pagination envelope is informational only; every field is
/// tolerant of being missing so a schema change on their side does not break
/// deserialization. Field names track the current API shape (`limit` /
/// `count`); the old `per_page` / `total` / `total_pages` were retired
/// upstream in 2026.
#[derive(Debug, Deserialize)]
pub struct MbPagination {
    #[serde(default)]
    #[allow(dead_code)]
    pub page: Option<i32>,
    #[serde(default)]
    #[allow(dead_code)]
    pub limit: Option<i32>,
    #[serde(default)]
    #[allow(dead_code)]
    pub count: Option<i32>,
    #[serde(default)]
    #[allow(dead_code)]
    pub next: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub previous: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MbSeries {
    pub id: i64,
    #[serde(default)]
    #[allow(dead_code)]
    pub state: Option<String>,
    pub title: String,
    #[serde(default)]
    pub native_title: Option<String>,
    #[serde(default)]
    pub romanized_title: Option<String>,
    /// MangaBaka returns this as `{ "en": [{type, title, note?}], "ja": [...] }`.
    #[serde(default)]
    pub secondary_titles: Option<serde_json::Value>,
    #[serde(default)]
    pub cover: Option<MbCover>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub year: Option<i32>,
    /// Last/final volume number. MangaBaka returns this as a nullable string.
    #[serde(default)]
    pub final_volume: Option<String>,
    /// Total chapter count. MangaBaka returns this as a nullable string.
    #[serde(default)]
    pub total_chapters: Option<String>,
    /// Average user rating on a 0-100 scale (e.g. `85.06` ≈ 8.5/10). May
    /// be missing for sparsely-reviewed series; the mapping layer divides
    /// by 10 so the canonical value is always on a 0-10 scale.
    #[serde(default)]
    pub rating: Option<f64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub genres: Option<Vec<String>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// `{anilist: {id, ...}, my_anime_list: {...}, mangadex: {...},
    ///  manga_updates: {...}, kitsu: {...}, anime_planet: {...},
    ///  anime_news_network: {...}, shikimori: {...}}`
    #[serde(default)]
    pub source: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MbCover {
    #[serde(default)]
    pub raw: Option<MbCoverRaw>,
    #[serde(default)]
    pub x350: Option<MbScaledImage>,
    #[serde(default)]
    pub x250: Option<MbScaledImage>,
    #[serde(default)]
    pub x150: Option<MbScaledImage>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MbCoverRaw {
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MbScaledImage {
    #[serde(default)]
    pub x1: Option<String>,
    #[serde(default)]
    pub x2: Option<String>,
    #[serde(default)]
    pub x3: Option<String>,
}

/// Thin HTTP client around the MangaBaka v1 REST API. Stateless apart from
/// the API key and HTTP client.
pub struct MangabakaClient {
    http: LimitedClient,
    base_url: String,
    api_key: Option<String>,
}

impl MangabakaClient {
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        timeout: Duration,
        limiter: Arc<HttpLimiter>,
    ) -> anyhow::Result<Self> {
        let inner = Client::builder()
            .timeout(timeout)
            .user_agent(concat!("tsundoku/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http: limiter.client(inner),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key,
        })
    }

    /// `GET /v1/series/{id}`. Returns `Ok(None)` on 404.
    pub async fn get_series(&self, id: &str) -> Result<Option<MbSeries>, MetadataError> {
        let url = format!("{}/v1/series/{}", self.base_url, urlencode(id));
        match self.request_envelope::<MbSeries>(&url).await? {
            EnvelopeOutcome::Ok(series) => Ok(Some(series)),
            EnvelopeOutcome::NotFound => Ok(None),
        }
    }

    /// `GET /v1/series/search?q=...&page=1&limit=N`. Returns the page of
    /// results; pagination is not exposed here (callers ask for one page).
    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<MbSeries>, MetadataError> {
        let url = format!(
            "{}/v1/series/search?q={}&page=1&limit={}",
            self.base_url,
            urlencode(query),
            limit
        );
        match self.request_envelope::<Vec<MbSeries>>(&url).await? {
            EnvelopeOutcome::Ok(rows) => Ok(rows),
            EnvelopeOutcome::NotFound => Ok(Vec::new()),
        }
    }

    /// `GET /v1/source/{source}/{id}`. Maps a foreign provider's ID to a
    /// MangaBaka series. Returns `Ok(None)` on 404 (no MangaBaka entry
    /// cross-references that foreign ID).
    pub async fn get_by_source_id(
        &self,
        source: &str,
        id: &str,
    ) -> Result<Option<MbSeries>, MetadataError> {
        let url = format!(
            "{}/v1/source/{}/{}",
            self.base_url,
            urlencode(source),
            urlencode(id)
        );
        match self.request_envelope::<MbSeries>(&url).await? {
            EnvelopeOutcome::Ok(series) => Ok(Some(series)),
            EnvelopeOutcome::NotFound => Ok(None),
        }
    }

    async fn request_envelope<T>(&self, url: &str) -> Result<EnvelopeOutcome<T>, MetadataError>
    where
        T: for<'de> serde::Deserialize<'de>,
    {
        let mut req = self.http.get(url);
        if let Some(key) = &self.api_key {
            req = req.header("x-api-key", key);
        }
        req = req.header("accept", "application/json");

        let resp = req.send().await.map_err(|e| MetadataError::Unavailable {
            provider: PROVIDER_ID.into(),
            source: anyhow!(e),
        })?;

        match resp.status() {
            StatusCode::OK => {
                let bytes = resp.bytes().await.map_err(|e| MetadataError::Unavailable {
                    provider: PROVIDER_ID.into(),
                    source: anyhow!(e),
                })?;
                let env: MbEnvelope<T> =
                    serde_json::from_slice(&bytes).map_err(|e| MetadataError::Malformed {
                        provider: PROVIDER_ID.into(),
                        message: format!("deserializing {url}: {e}"),
                    })?;
                Ok(EnvelopeOutcome::Ok(env.data))
            }
            StatusCode::NOT_FOUND => Ok(EnvelopeOutcome::NotFound),
            StatusCode::TOO_MANY_REQUESTS => {
                let retry_after = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(Duration::from_secs);
                Err(MetadataError::RateLimited {
                    provider: PROVIDER_ID.into(),
                    retry_after,
                })
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(MetadataError::AuthFailed {
                provider: PROVIDER_ID.into(),
            }),
            other => Err(MetadataError::Unavailable {
                provider: PROVIDER_ID.into(),
                source: anyhow!("HTTP {} from {}", other.as_u16(), url),
            }),
        }
    }
}

enum EnvelopeOutcome<T> {
    Ok(T),
    NotFound,
}

/// Minimal URL component encoder. MangaBaka source ids are numeric and slug
/// names are ASCII identifiers, so a full URL encoder is overkill; we just
/// percent-encode the characters that would break path parsing.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_deserializes_single_series() {
        let raw = r#"{
            "status": 200,
            "data": {
                "id": 1,
                "title": "Chainsaw Man",
                "year": 2018,
                "type": "manga",
                "status": "releasing",
                "source": {
                    "anilist": { "id": 105778 },
                    "my_anime_list": { "id": 116778 }
                }
            }
        }"#;
        let env: MbEnvelope<MbSeries> = serde_json::from_str(raw).unwrap();
        assert_eq!(env.data.id, 1);
        assert_eq!(env.data.title, "Chainsaw Man");
        assert_eq!(env.data.year, Some(2018));
        assert_eq!(env.data.kind.as_deref(), Some("manga"));
    }

    #[test]
    fn envelope_deserializes_search_results() {
        let raw = r#"{
            "status": 200,
            "data": [
                {"id": 1, "title": "Chainsaw Man"},
                {"id": 2, "title": "Berserk"}
            ],
            "pagination": { "count": 2, "next": null, "previous": null, "page": 1, "limit": 20 }
        }"#;
        let env: MbEnvelope<Vec<MbSeries>> = serde_json::from_str(raw).unwrap();
        assert_eq!(env.data.len(), 2);
        assert_eq!(env.data[0].title, "Chainsaw Man");
    }

    #[test]
    fn urlencode_passes_safe_chars_through() {
        assert_eq!(urlencode("123"), "123");
        assert_eq!(urlencode("manga-updates"), "manga-updates");
        assert_eq!(urlencode("hello world"), "hello%20world");
        assert_eq!(urlencode("a/b"), "a%2Fb");
    }
}
