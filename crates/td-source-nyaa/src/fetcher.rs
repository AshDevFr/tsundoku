//! Conditional GET fetcher for the Nyaa RSS feed and per-post detail pages.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use reqwest::{Client, StatusCode};
use td_http::{HttpLimiter, LimitedClient};

pub struct Fetcher {
    http: LimitedClient,
}

/// Outcome of a conditional feed fetch.
pub enum FetcherResult {
    /// Upstream returned 304 Not Modified; the body is unchanged from the
    /// previous fetch. New ETag (if any) is propagated for next time.
    NotModified { etag: Option<String> },
    /// Upstream returned a fresh body.
    Body { body: String, etag: Option<String> },
}

impl Fetcher {
    pub fn new(timeout: Duration, limiter: Arc<HttpLimiter>) -> Result<Self> {
        let inner = Client::builder()
            .timeout(timeout)
            .user_agent(concat!("tsundoku/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http: limiter.client(inner),
        })
    }

    pub async fn fetch_feed(
        &self,
        url: &str,
        if_none_match: Option<&str>,
    ) -> Result<FetcherResult> {
        let mut req = self.http.get(url).header("accept", "application/rss+xml");
        if let Some(etag) = if_none_match {
            req = req.header("if-none-match", etag);
        }
        let resp = req.send().await.with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|h| h.to_str().ok())
            .map(str::to_string);
        match status {
            StatusCode::NOT_MODIFIED => Ok(FetcherResult::NotModified { etag }),
            StatusCode::OK => {
                let body = resp
                    .text()
                    .await
                    .with_context(|| format!("reading body from {url}"))?;
                Ok(FetcherResult::Body { body, etag })
            }
            other => Err(anyhow!("HTTP {} from {url}", other.as_u16())),
        }
    }

    pub async fn fetch_detail(&self, url: &str) -> Result<String> {
        let resp = self
            .http
            .get(url)
            .header("accept", "text/html")
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("HTTP {} from {url}", resp.status().as_u16()));
        }
        resp.text()
            .await
            .with_context(|| format!("reading detail body from {url}"))
    }
}
