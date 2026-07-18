//! [`td_source::SearchSource`] impl for Nyaa: on-demand title search over
//! the paginated HTML listing.
//!
//! Search reuses the backfill machinery wholesale: the results page at
//! `?q=<title>&p=N` is the same listing table [`crate::listing`] already
//! parses, and the RSS view is useless here for the same reason it is for
//! backfill (it silently ignores `&p=N`). Only URL construction differs:
//! the configured `search_url` carries the category/filter params and this
//! module appends `q` and `p`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use td_http::HttpLimiter;
use td_source::{DiscoveredRelease, SearchSource, SourceError, SourceResult};

use crate::SOURCE_KIND;
use crate::fetcher::Fetcher;
use crate::listing;
use crate::source::enrich_from_detail;

/// Per-entry config consumed by [`NyaaSearch::from_config`]. Mirrors the
/// `[[search]]` block when `kind = "nyaa"`.
#[derive(Debug, Clone)]
pub struct NyaaSearchConfig {
    /// `[[search]]` entry name; stamped as `source_name` on hits.
    pub name: String,
    /// Paginated HTML listing URL with filters baked in (no `q`/`p`).
    pub search_url: String,
    /// HTTP timeout per request (listing and detail fetches).
    pub timeout: Duration,
    /// Whether to fetch each hit's detail page during enrich.
    pub fetch_details: bool,
    /// Base URL for relative `/view/N` and `/download/N.torrent` hrefs.
    pub site_base_url: String,
}

pub struct NyaaSearch {
    cfg: NyaaSearchConfig,
    fetcher: Fetcher,
}

impl NyaaSearch {
    pub fn from_config(
        cfg: NyaaSearchConfig,
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
impl SearchSource for NyaaSearch {
    fn name(&self) -> &str {
        &self.cfg.name
    }

    fn kind(&self) -> &str {
        SOURCE_KIND
    }

    async fn search_page(&self, query: &str, page: u32) -> SourceResult<Vec<DiscoveredRelease>> {
        let url = search_page_url(&self.cfg.search_url, query, page)
            .map_err(|e| self.malformed(format!("building search url: {e}")))?;
        let html = self
            .fetcher
            .fetch_listing(&url)
            .await
            .map_err(|e| self.unavailable(e))?;
        let releases = listing::parse_listing(&html, &self.cfg.name, &self.cfg.site_base_url)
            .map_err(|e| self.malformed(format!("parsing search page {page}: {e}")))?;
        tracing::info!(
            search = %self.cfg.name,
            query,
            page,
            url = %url,
            count = releases.len(),
            "parsed nyaa search page"
        );
        Ok(releases)
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
}

/// Build the results URL for one (query, page) pair: keep the configured
/// filter params, drop any stale `q`/`p`/`page=rss` leftovers (tolerating
/// an operator who pasted a full search or RSS URL), then append the
/// percent-encoded `q` and, past page 1, `p`. Page 1 omits `p` because
/// Nyaa renders it identically — keeps the URL stable for cache hits.
pub(crate) fn search_page_url(search_url: &str, query: &str, page: u32) -> anyhow::Result<String> {
    let mut url = url::Url::parse(search_url)
        .with_context(|| format!("invalid search_url {search_url:?}"))?;
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, v)| !(k == "q" || k == "p" || (k == "page" && v == "rss")))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    {
        let mut pairs = url.query_pairs_mut();
        pairs.clear();
        for (k, v) in &kept {
            pairs.append_pair(k, v);
        }
        pairs.append_pair("q", query);
        if page > 1 {
            pairs.append_pair("p", &page.to_string());
        }
    }
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::search_page_url;

    const RESULTS_FIXTURE: &str = include_str!("../tests/fixtures/nyaa_search_results.html");
    const EMPTY_FIXTURE: &str = include_str!("../tests/fixtures/nyaa_search_empty.html");

    #[test]
    fn url_appends_query_and_keeps_filters() {
        assert_eq!(
            search_page_url("https://nyaa.si/?f=0&c=3_1", "frieren", 1).unwrap(),
            "https://nyaa.si/?f=0&c=3_1&q=frieren"
        );
    }

    #[test]
    fn url_appends_p_past_page_one() {
        assert_eq!(
            search_page_url("https://nyaa.si/?f=0&c=3_1", "frieren", 3).unwrap(),
            "https://nyaa.si/?f=0&c=3_1&q=frieren&p=3"
        );
    }

    #[test]
    fn url_percent_encodes_the_query() {
        assert_eq!(
            search_page_url("https://nyaa.si/?c=3_1", "one piece", 1).unwrap(),
            "https://nyaa.si/?c=3_1&q=one+piece"
        );
        // Non-Latin titles survive as UTF-8 percent-encoding; they matter
        // against raw-category entries.
        assert_eq!(
            search_page_url("https://nyaa.si/?c=3_3", "葬送のフリーレン", 1).unwrap(),
            "https://nyaa.si/?c=3_3&q=%E8%91%AC%E9%80%81%E3%81%AE%E3%83%95%E3%83%AA%E3%83%BC%E3%83%AC%E3%83%B3"
        );
    }

    #[test]
    fn url_drops_stale_q_p_and_rss_switch() {
        // Operator pasted a full search/RSS URL: still works.
        assert_eq!(
            search_page_url("https://nyaa.si/?page=rss&q=old&p=9&c=3_1", "new", 2).unwrap(),
            "https://nyaa.si/?c=3_1&q=new&p=2"
        );
    }

    #[test]
    fn url_rejects_garbage() {
        assert!(search_page_url("not a url", "x", 1).is_err());
    }

    #[test]
    fn search_results_fixture_parses_into_releases() {
        let releases =
            crate::listing::parse_listing(RESULTS_FIXTURE, "nyaa-eng", "https://nyaa.si").unwrap();
        assert_eq!(releases.len(), 52, "recorded fixture has 52 result rows");
        let first = &releases[0];
        assert_eq!(first.source_name, "nyaa-eng");
        assert!(!first.external_id.is_empty());
        assert!(first.title.to_lowercase().contains("frieren"));
        assert!(first.link.starts_with("https://nyaa.si/view/"));
    }

    #[test]
    fn empty_search_fixture_parses_into_no_releases() {
        let releases =
            crate::listing::parse_listing(EMPTY_FIXTURE, "nyaa-eng", "https://nyaa.si").unwrap();
        assert!(releases.is_empty());
    }
}
