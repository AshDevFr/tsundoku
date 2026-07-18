//! `DiscoverySource` impl for Nyaa.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use td_http::HttpLimiter;
use td_source::{
    Backfillable, DiscoveredRelease, DiscoverySource, PollContext, PollOutcome, SourceError,
    SourceResult,
};

use crate::SOURCE_KIND;
use crate::fetcher::{Fetcher, FetcherResult};
use crate::{listing, parser};

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
        // Nyaa's RSS view (`?page=rss`) does not honour `&p=N` — every
        // page request returns the same most-recent 75 items. The poll is
        // therefore single-fetch by construction. Historical catch-up
        // belongs in [`Backfillable::backfill_page`], which walks the
        // paginated HTML listing instead.
        let fetched = self
            .fetcher
            .fetch_feed(&self.cfg.feed_url, ctx.etag.as_deref())
            .await
            .map_err(|e| self.unavailable(e))?;

        let (body, new_etag) = match fetched {
            FetcherResult::NotModified { etag } => {
                tracing::info!(
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

        let survivors = self.parse_and_filter(&body, &ctx.recently_seen)?;
        tracing::info!(
            source = %self.cfg.name,
            new = survivors.len(),
            "parsed nyaa feed"
        );

        Ok(PollOutcome {
            releases: survivors,
            new_etag,
            new_cursor: None,
            not_modified: false,
        })
    }

    async fn enrich(&self, release: &mut DiscoveredRelease) -> SourceResult<()> {
        if !self.cfg.fetch_details {
            return Ok(());
        }
        enrich_from_detail(
            &self.fetcher,
            &self.cfg.name,
            &self.cfg.site_base_url,
            release,
        )
        .await;
        Ok(())
    }

    fn as_backfillable(&self) -> Option<&dyn Backfillable> {
        Some(self)
    }
}

#[async_trait]
impl Backfillable for NyaaSource {
    async fn backfill_page(&self, page: u32) -> SourceResult<Vec<DiscoveredRelease>> {
        let url = listing_url(&self.cfg.feed_url, page);
        let html = self
            .fetcher
            .fetch_listing(&url)
            .await
            .map_err(|e| self.unavailable(e))?;
        let releases = listing::parse_listing(&html, &self.cfg.name, &self.cfg.site_base_url)
            .map_err(|e| self.malformed(format!("parsing listing page {page}: {e}")))?;
        tracing::info!(
            source = %self.cfg.name,
            page,
            url = %url,
            count = releases.len(),
            "parsed nyaa listing page"
        );
        Ok(releases)
    }
}

/// Fetch + parse a post's detail page and fold the richer fields into
/// `release`. Shared by the poll path ([`NyaaSource::enrich`]) and the
/// per-series search path ([`crate::search::NyaaSearch`]).
///
/// Infallible by design: enrich failures are non-fatal by trait contract,
/// so any fetch/parse problem is logged and the release keeps whatever
/// data the feed/listing pass already provided. A flaky detail-page host
/// shouldn't sink a poll or a search.
pub(crate) async fn enrich_from_detail(
    fetcher: &Fetcher,
    source_name: &str,
    site_base_url: &str,
    release: &mut DiscoveredRelease,
) {
    let html = match fetcher.fetch_detail(&release.link).await {
        Ok(html) => html,
        Err(e) => {
            tracing::warn!(
                source = %source_name,
                link = %release.link,
                error = ?e,
                "failed to fetch nyaa detail page; keeping feed-only data"
            );
            return;
        }
    };
    let detail = match crate::detail::parse_detail(&html, site_base_url) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(
                source = %source_name,
                link = %release.link,
                error = %e,
                "failed to parse nyaa detail page; keeping feed-only data"
            );
            return;
        }
    };
    if !detail.files.is_empty() {
        release.files = detail.files;
    }
    if !detail.external_links.is_empty() {
        release.external_links = detail.external_links;
    }
    if !detail.comment_suggested_links.is_empty() {
        release.comment_suggested_links = detail.comment_suggested_links;
    }
    if detail.information_url.is_some() {
        release.information_url = detail.information_url;
    }
    if release.magnet.is_none() {
        release.magnet = detail.magnet;
    }
    // The feed/listing gives a short anchor + size + category + hash; the
    // detail page has the uploader's actual body (markdown). Prefer the
    // latter when present — it's what the review UI surfaces.
    if let Some(desc) = detail.description_html {
        release.description_html = Some(desc);
    }
}

/// Derive the HTML listing URL for `page` from the configured RSS
/// `feed_url`: strip `page=rss` (the RSS-mode switch) and append `&p=N`.
/// Any pre-existing `p=` segment is replaced. Page 1 omits `&p=` because
/// Nyaa renders it identically — keeps the URL stable for cache hits.
pub(crate) fn listing_url(feed_url: &str, page: u32) -> String {
    let (path_and_query, fragment) = match feed_url.split_once('#') {
        Some((p, f)) => (p.to_string(), Some(f.to_string())),
        None => (feed_url.to_string(), None),
    };
    let (prefix, query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (path_and_query, String::new()),
    };
    let mut kept: Vec<String> = query
        .split('&')
        .filter(|seg| !seg.is_empty() && *seg != "page=rss" && !seg.starts_with("p="))
        .map(str::to_string)
        .collect();
    if page > 1 {
        kept.push(format!("p={page}"));
    }
    let mut out = prefix;
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    if let Some(frag) = fragment {
        out.push('#');
        out.push_str(&frag);
    }
    out
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

#[cfg(test)]
mod tests {
    use super::listing_url;

    #[test]
    fn listing_url_strips_page_rss_and_omits_p_for_page_one() {
        assert_eq!(
            listing_url("https://nyaa.si/?page=rss&c=3_1&f=2", 1),
            "https://nyaa.si/?c=3_1&f=2"
        );
    }

    #[test]
    fn listing_url_appends_p_for_higher_pages() {
        assert_eq!(
            listing_url("https://nyaa.si/?page=rss&c=3_1&f=2", 2),
            "https://nyaa.si/?c=3_1&f=2&p=2"
        );
    }

    #[test]
    fn listing_url_replaces_existing_p() {
        assert_eq!(
            listing_url("https://nyaa.si/?page=rss&p=7&c=3_1", 3),
            "https://nyaa.si/?c=3_1&p=3"
        );
    }

    #[test]
    fn listing_url_handles_no_query_string() {
        assert_eq!(listing_url("https://nyaa.si/", 1), "https://nyaa.si/");
        assert_eq!(listing_url("https://nyaa.si/", 4), "https://nyaa.si/?p=4");
    }

    #[test]
    fn listing_url_preserves_fragment() {
        assert_eq!(
            listing_url("https://nyaa.si/?page=rss#top", 2),
            "https://nyaa.si/?p=2#top"
        );
    }
}
