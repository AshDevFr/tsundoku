//! `DiscoverySource` impl for Nyaa.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use td_http::HttpLimiter;
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
    /// Maximum number of feed pages to walk per poll. `1` preserves the
    /// single-request steady-state behaviour; higher values let the source
    /// catch up after downtime. ETag short-circuits the whole loop on page
    /// 1; per-page items already in `PollContext.recently_seen` are dropped
    /// before the detail-fetch pass.
    pub max_pages: u32,
}

impl Default for NyaaSourceConfig {
    fn default() -> Self {
        Self {
            name: "default".into(),
            feed_url: "https://nyaa.si/?page=rss".into(),
            timeout: Duration::from_secs(30),
            fetch_details: false,
            site_base_url: "https://nyaa.si".into(),
            max_pages: 1,
        }
    }
}

pub struct NyaaSource {
    cfg: NyaaSourceConfig,
    fetcher: Fetcher,
}

impl NyaaSource {
    pub fn from_config(
        cfg: NyaaSourceConfig,
        limiter: Arc<HttpLimiter>,
    ) -> Result<Self, SourceError> {
        let fetcher =
            Fetcher::new(cfg.timeout, limiter).map_err(|e| SourceError::NotConfigured {
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
        // Page 1 with the previous ETag. A 304 short-circuits the entire
        // multi-page walk: if the first page hasn't changed, the later
        // pages are stale by construction.
        let first = self
            .fetcher
            .fetch_feed(&self.cfg.feed_url, ctx.etag.as_deref())
            .await
            .map_err(|e| self.unavailable(e))?;

        let (body, new_etag) = match first {
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

        let mut survivors = self.parse_and_filter(&body, &ctx.recently_seen)?;

        let max_pages = self.cfg.max_pages.max(1);
        for page in 2..=max_pages {
            let url = page_url(&self.cfg.feed_url, page);
            // Subsequent pages: no ETag (the previous one was bound to page
            // 1's URL). A failure on page N doesn't poison page 1's results
            // — log and stop walking forward.
            let fetched = match self.fetcher.fetch_feed(&url, None).await {
                Ok(FetcherResult::Body { body, .. }) => body,
                Ok(FetcherResult::NotModified { .. }) => continue,
                Err(e) => {
                    tracing::warn!(
                        source = %self.cfg.name,
                        page,
                        url = %url,
                        error = ?e,
                        "failed to fetch nyaa page; stopping pagination walk"
                    );
                    break;
                }
            };
            match self.parse_and_filter(&fetched, &ctx.recently_seen) {
                Ok(mut page_survivors) => survivors.append(&mut page_survivors),
                Err(e) => {
                    tracing::warn!(
                        source = %self.cfg.name,
                        page,
                        error = ?e,
                        "failed to parse nyaa page; stopping pagination walk"
                    );
                    break;
                }
            }
        }

        if self.cfg.fetch_details {
            for release in survivors.iter_mut() {
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
            releases: survivors,
            new_etag,
            new_cursor: None,
            not_modified: false,
        })
    }
}

impl NyaaSource {
    fn parse_and_filter(
        &self,
        body: &str,
        recently_seen: &std::collections::HashSet<String>,
    ) -> SourceResult<Vec<td_source::DiscoveredRelease>> {
        let mut releases = parser::parse_feed(body, &self.cfg.name)
            .map_err(|e| self.malformed(format!("parsing rss feed: {e}")))?;
        if !recently_seen.is_empty() {
            releases.retain(|r| !recently_seen.contains(&r.external_id));
        }
        Ok(releases)
    }
}

/// Build the URL for page `page` of the feed. Page 1 is the base URL
/// unchanged; for higher pages, any existing `p=` query param is stripped
/// and replaced with the requested page number. Nyaa accepts `p=N` on the
/// same URL that carries `page=rss` (e.g.
/// `https://nyaa.si/?page=rss&c=3_1&p=2`).
fn page_url(base: &str, page: u32) -> String {
    if page <= 1 {
        return base.to_string();
    }
    let (path, fragment) = match base.split_once('#') {
        Some((p, f)) => (p.to_string(), Some(f.to_string())),
        None => (base.to_string(), None),
    };
    let (prefix, query) = match path.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (path, String::new()),
    };
    let mut kept: Vec<&str> = query
        .split('&')
        .filter(|seg| !seg.is_empty() && !seg.starts_with("p="))
        .collect();
    let page_seg = format!("p={page}");
    kept.push(&page_seg);
    let mut out = prefix;
    out.push('?');
    out.push_str(&kept.join("&"));
    if let Some(frag) = fragment {
        out.push('#');
        out.push_str(&frag);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_url_appends_param_when_absent() {
        assert_eq!(
            page_url("https://nyaa.si/?page=rss&c=3_1", 2),
            "https://nyaa.si/?page=rss&c=3_1&p=2"
        );
    }

    #[test]
    fn page_url_replaces_existing_param() {
        assert_eq!(
            page_url("https://nyaa.si/?page=rss&p=5&c=3_1", 3),
            "https://nyaa.si/?page=rss&c=3_1&p=3"
        );
    }

    #[test]
    fn page_url_keeps_base_for_page_one() {
        let base = "https://nyaa.si/?page=rss&c=3_1";
        assert_eq!(page_url(base, 1), base);
    }

    #[test]
    fn page_url_handles_no_existing_query_string() {
        assert_eq!(page_url("https://nyaa.si/", 4), "https://nyaa.si/?p=4");
    }

    #[test]
    fn page_url_preserves_fragment() {
        assert_eq!(
            page_url("https://nyaa.si/?page=rss#top", 2),
            "https://nyaa.si/?page=rss&p=2#top"
        );
    }
}
