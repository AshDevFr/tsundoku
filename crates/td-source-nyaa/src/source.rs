//! `DiscoverySource` impl for Nyaa.

use std::time::Duration;

use async_trait::async_trait;
use td_source::{DiscoverySource, PollContext, PollOutcome, SourceError, SourceResult};

use crate::SOURCE_KIND;
use crate::fetcher::{Fetcher, FetcherResult};
use crate::parser;

/// Per-instance config consumed by [`NyaaSource::from_config`]. Mirrors the
/// `[[sources]]` block when `kind = "nyaa"`.
#[derive(Debug, Clone)]
pub struct NyaaSourceConfig {
    pub name: String,
    pub feed_url: String,
    /// HTTP timeout per request. Applied to both the feed fetch and any
    /// detail-page fetches that follow.
    pub timeout: Duration,
    /// Whether to fetch each post's detail page to enrich `files` and
    /// extracted external links. Off by default (kept cheap); the resolver
    /// runs fine on description-only data.
    pub fetch_details: bool,
    /// Optional override for the detail base URL (e.g. when the feed is
    /// proxied). Defaults to `https://nyaa.si`.
    pub site_base_url: String,
}

impl Default for NyaaSourceConfig {
    fn default() -> Self {
        Self {
            name: "default".into(),
            feed_url: "https://nyaa.si/?page=rss".into(),
            timeout: Duration::from_secs(30),
            fetch_details: false,
            site_base_url: "https://nyaa.si".into(),
        }
    }
}

pub struct NyaaSource {
    cfg: NyaaSourceConfig,
    fetcher: Fetcher,
}

impl NyaaSource {
    pub fn from_config(cfg: NyaaSourceConfig) -> Result<Self, SourceError> {
        let fetcher = Fetcher::new(cfg.timeout).map_err(|e| SourceError::NotConfigured {
            source_kind: SOURCE_KIND.into(),
            source_name: cfg.name.clone(),
            message: format!("building http client: {e}"),
        })?;
        Ok(Self { cfg, fetcher })
    }

    fn unavailable(&self, err: anyhow::Error) -> SourceError {
        SourceError::Unavailable {
            source_kind: SOURCE_KIND.into(),
            source_name: self.cfg.name.clone(),
            source: err,
        }
    }

    fn malformed(&self, message: impl Into<String>) -> SourceError {
        SourceError::Malformed {
            source_kind: SOURCE_KIND.into(),
            source_name: self.cfg.name.clone(),
            message: message.into(),
        }
    }
}

#[async_trait]
impl DiscoverySource for NyaaSource {
    fn name(&self) -> &str {
        &self.cfg.name
    }

    fn kind(&self) -> &str {
        SOURCE_KIND
    }

    async fn poll(&self, ctx: &PollContext) -> SourceResult<PollOutcome> {
        let fetched = self
            .fetcher
            .fetch_feed(&self.cfg.feed_url, ctx.etag.as_deref())
            .await
            .map_err(|e| self.unavailable(e))?;

        let (body, new_etag) = match fetched {
            FetcherResult::NotModified { etag } => {
                tracing::debug!(
                    source = %self.cfg.name,
                    "nyaa feed returned 304 Not Modified"
                );
                return Ok(PollOutcome {
                    releases: Vec::new(),
                    new_etag: etag,
                    new_cursor: None,
                    not_modified: true,
                });
            }
            FetcherResult::Body { body, etag } => (body, etag),
        };

        let mut releases = parser::parse_feed(&body, &self.cfg.name)
            .map_err(|e| self.malformed(format!("parsing rss feed: {e}")))?;

        if self.cfg.fetch_details {
            for release in releases.iter_mut() {
                match self
                    .fetcher
                    .fetch_detail(&release.link)
                    .await
                    .map(|html| crate::detail::parse_detail(&html, &self.cfg.site_base_url))
                {
                    Ok(Ok(detail)) => {
                        if !detail.files.is_empty() {
                            release.files = detail.files;
                        }
                        if !detail.external_links.is_empty() {
                            release.external_links = detail.external_links;
                        }
                        if release.magnet.is_none() {
                            release.magnet = detail.magnet;
                        }
                        // RSS gives us a short anchor + size + category +
                        // hash; the detail page has the uploader's actual
                        // body (markdown). Prefer the latter when present —
                        // it's what the review UI surfaces to the operator.
                        if let Some(desc) = detail.description_html {
                            release.description_html = Some(desc);
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(
                            source = %self.cfg.name,
                            link = %release.link,
                            error = %e,
                            "failed to parse nyaa detail page; keeping rss-only data"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            source = %self.cfg.name,
                            link = %release.link,
                            error = ?e,
                            "failed to fetch nyaa detail page; keeping rss-only data"
                        );
                    }
                }
            }
        }

        Ok(PollOutcome {
            releases,
            new_etag,
            new_cursor: None,
            not_modified: false,
        })
    }
}
