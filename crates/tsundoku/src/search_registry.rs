//! Release-search registry construction.
//!
//! Mirrors [`crate::source_registry`]: reads the `[[search]]` array from
//! config and dispatches each entry to its kind-specific constructor.
//! Called at `serve` boot even before anything uses the registry, so a
//! broken entry fails the launch rather than the first button click.
//!
//! Adding a new search kind = importing its crate and adding one arm to
//! the `match`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow};
use td_config::{AppConfig, SearchEntryConfig};
use td_http::HttpLimiter;
use td_source::{SearchEntry, SearchRegistry, SearchSource};
use td_source_nyaa::{NyaaSearch, NyaaSearchConfig};

pub fn build_search_registry(
    cfg: &AppConfig,
    limiter: Arc<HttpLimiter>,
) -> anyhow::Result<SearchRegistry> {
    let mut builder = SearchRegistry::builder();
    for entry in &cfg.search {
        if !entry.enabled {
            tracing::info!(search = %entry.name, "skipping disabled search entry");
            continue;
        }
        let source: Arc<dyn SearchSource> = construct_search(entry, limiter.clone())?;
        builder
            .register(SearchEntry {
                source,
                is_default: entry.is_default,
                max_pages: entry.max_pages.max(1),
            })
            .with_context(|| format!("registering search entry {:?}", entry.name))?;
    }
    Ok(builder.build())
}

fn construct_search(
    entry: &SearchEntryConfig,
    limiter: Arc<HttpLimiter>,
) -> anyhow::Result<Arc<dyn SearchSource>> {
    match entry.kind.as_str() {
        td_source_nyaa::SOURCE_KIND => {
            let opts = entry.nyaa.as_ref().ok_or_else(|| {
                anyhow!(
                    "search entry {:?} (kind=nyaa) is missing the [search.nyaa] options block",
                    entry.name
                )
            })?;
            if opts.search_url.is_empty() {
                return Err(anyhow!(
                    "search entry {:?} (kind=nyaa) requires [search.nyaa].search_url",
                    entry.name
                ));
            }
            let nyaa_cfg = NyaaSearchConfig {
                name: entry.name.clone(),
                search_url: opts.search_url.clone(),
                timeout: Duration::from_secs(opts.timeout_seconds.max(1) as u64),
                fetch_details: opts.fetch_details,
                site_base_url: opts.site_base_url.clone(),
            };
            let nyaa = NyaaSearch::from_config(nyaa_cfg, limiter)
                .with_context(|| format!("building nyaa search entry {:?}", entry.name))?;
            Ok(Arc::new(nyaa))
        }
        other => Err(anyhow!(
            "search entry {:?} has unknown kind {other:?} (only \"nyaa\" is supported)",
            entry.name
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use td_config::NyaaSearchOptions;

    fn limiter() -> Arc<HttpLimiter> {
        HttpLimiter::no_limit()
    }

    fn entry(name: &str, is_default: bool, enabled: bool) -> SearchEntryConfig {
        SearchEntryConfig {
            kind: "nyaa".into(),
            name: name.into(),
            is_default,
            enabled,
            max_pages: 5,
            nyaa: Some(NyaaSearchOptions {
                search_url: "https://nyaa.si/?f=0&c=3_1".into(),
                ..Default::default()
            }),
        }
    }

    fn config_with(entries: Vec<SearchEntryConfig>) -> AppConfig {
        AppConfig {
            search: entries,
            ..Default::default()
        }
    }

    #[test]
    fn builds_entries_and_resolves_default() {
        let cfg = config_with(vec![entry("eng", false, true), entry("raw", true, true)]);
        let reg = build_search_registry(&cfg, limiter()).unwrap();
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.default_entry().unwrap().source.name(), "raw");
        assert_eq!(reg.get("eng").unwrap().source.kind(), "nyaa");
    }

    #[test]
    fn skips_disabled_entries() {
        let cfg = config_with(vec![entry("eng", false, true), entry("off", false, false)]);
        let reg = build_search_registry(&cfg, limiter()).unwrap();
        assert_eq!(reg.len(), 1);
        assert!(reg.get("off").is_none());
        // With nothing marked, the first enabled entry is the default.
        assert_eq!(reg.default_entry().unwrap().source.name(), "eng");
    }

    #[test]
    fn clamps_max_pages_to_at_least_one() {
        let mut e = entry("eng", false, true);
        e.max_pages = 0;
        let reg = build_search_registry(&config_with(vec![e]), limiter()).unwrap();
        assert_eq!(reg.get("eng").unwrap().max_pages, 1);
    }

    #[test]
    fn missing_options_block_fails_construction() {
        let mut e = entry("eng", false, true);
        e.nyaa = None;
        match build_search_registry(&config_with(vec![e]), limiter()) {
            Err(err) => assert!(err.to_string().contains("[search.nyaa]")),
            Ok(_) => panic!("expected missing options block to fail construction"),
        }
    }

    #[test]
    fn empty_config_builds_empty_registry() {
        let reg = build_search_registry(&AppConfig::default(), limiter()).unwrap();
        assert!(reg.is_empty());
        assert!(reg.default_entry().is_none());
    }
}
