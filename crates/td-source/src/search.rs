//! Per-series release search: the [`SearchSource`] trait + its registry.
//!
//! A search source answers "what does this upstream have for this title?"
//! on demand, unlike [`crate::DiscoverySource`] which polls a fixed feed on
//! a schedule. Instances are built from `[[search]]` config entries, not
//! from `[[sources]]`: a series doesn't know which source discovered it
//! (and the prime search target, a wishlisted or orphan series, has no
//! releases at all), so search endpoints are their own named concept.
//!
//! Hits flow through the same enrich → persist → resolve pipeline as poll
//! output. They are deliberately *not* force-linked to the series the
//! search was launched from: upstream search is substring-ish, so results
//! can be unrelated; the resolver and review queue sort that out.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::SourceResult;
use crate::release::DiscoveredRelease;

/// An upstream that can be queried for releases by free-text title.
#[async_trait]
pub trait SearchSource: Send + Sync {
    /// `[[search]]` entry name. Stamped as `source_name` on releases this
    /// search discovers first (already-known releases keep the name of
    /// whichever source saw them first; dedup is on
    /// `(source_kind, external_id)`).
    fn name(&self) -> &str;

    /// Search kind (e.g. `"nyaa"`). Persisted as `source_kind`, which is
    /// what lets search hits dedupe against feed-polled releases.
    fn kind(&self) -> &str;

    /// Fetch one page of hits for `query`. `page` is 1-indexed. Returning
    /// an empty Vec means "no more pages for this query"; the caller stops
    /// walking. Callers are also expected to cap the walk with the entry's
    /// configured `max_pages`.
    async fn search_page(&self, query: &str, page: u32) -> SourceResult<Vec<DiscoveredRelease>>;

    /// Optional per-release enrichment hook, same contract as
    /// [`crate::DiscoverySource::enrich`]: called before each hit is
    /// persisted, failures must be non-fatal (log and return `Ok`).
    async fn enrich(&self, _release: &mut DiscoveredRelease) -> SourceResult<()> {
        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SearchRegistryError {
    #[error("search entry named {0:?} registered more than once")]
    DuplicateName(String),
}

/// One registered search endpoint plus the per-entry policy the run engine
/// enforces (the trait stays pure upstream behavior).
pub struct SearchEntry {
    pub source: Arc<dyn SearchSource>,
    /// Marked `default = true` in config. [`SearchRegistry::default_entry`]
    /// falls back to the first registered entry when nothing is marked.
    pub is_default: bool,
    /// Per-query pagination cap from config.
    pub max_pages: u32,
}

/// Registry of search endpoints, built once at startup from the
/// `[[search]]` config array. Registration order is preserved: it is the
/// UI's dropdown order and the default fallback order.
pub struct SearchRegistry {
    entries: Vec<SearchEntry>,
}

impl SearchRegistry {
    pub fn builder() -> SearchRegistryBuilder {
        SearchRegistryBuilder::default()
    }

    /// Entry count is single digits in practice, so lookups scan.
    pub fn get(&self, name: &str) -> Option<&SearchEntry> {
        self.entries.iter().find(|e| e.source.name() == name)
    }

    /// The split button's primary action: the entry marked default, or the
    /// first registered one when none is marked. `None` only when the
    /// registry is empty.
    pub fn default_entry(&self) -> Option<&SearchEntry> {
        self.entries
            .iter()
            .find(|e| e.is_default)
            .or_else(|| self.entries.first())
    }

    pub fn iter(&self) -> impl Iterator<Item = &SearchEntry> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Default)]
pub struct SearchRegistryBuilder {
    entries: Vec<SearchEntry>,
}

impl SearchRegistryBuilder {
    /// Register an entry. Names must be unique across the registry: the
    /// trigger endpoint and the per-entry job lock both key on name alone.
    pub fn register(&mut self, entry: SearchEntry) -> Result<&mut Self, SearchRegistryError> {
        let name = entry.source.name();
        if self.entries.iter().any(|e| e.source.name() == name) {
            return Err(SearchRegistryError::DuplicateName(name.to_string()));
        }
        self.entries.push(entry);
        Ok(self)
    }

    pub fn build(self) -> SearchRegistry {
        SearchRegistry {
            entries: self.entries,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubSearch {
        name: &'static str,
    }

    #[async_trait]
    impl SearchSource for StubSearch {
        fn name(&self) -> &str {
            self.name
        }
        fn kind(&self) -> &str {
            "nyaa"
        }
        async fn search_page(
            &self,
            _query: &str,
            _page: u32,
        ) -> SourceResult<Vec<DiscoveredRelease>> {
            Ok(Vec::new())
        }
    }

    fn entry(name: &'static str, is_default: bool) -> SearchEntry {
        SearchEntry {
            source: Arc::new(StubSearch { name }),
            is_default,
            max_pages: 5,
        }
    }

    #[test]
    fn registers_and_looks_up_entries_by_name() {
        let mut b = SearchRegistry::builder();
        b.register(entry("eng", true)).unwrap();
        b.register(entry("raw", false)).unwrap();
        let reg = b.build();
        assert_eq!(reg.len(), 2);
        assert!(reg.get("eng").is_some());
        assert!(reg.get("raw").is_some());
        assert!(reg.get("nope").is_none());
    }

    #[test]
    fn rejects_duplicate_names() {
        let mut b = SearchRegistry::builder();
        b.register(entry("eng", false)).unwrap();
        match b.register(entry("eng", false)) {
            Err(SearchRegistryError::DuplicateName(n)) => assert_eq!(n, "eng"),
            Ok(_) => panic!("expected duplicate-name registration to fail"),
        }
    }

    #[test]
    fn default_entry_prefers_the_marked_entry() {
        let mut b = SearchRegistry::builder();
        b.register(entry("eng", false)).unwrap();
        b.register(entry("raw", true)).unwrap();
        let reg = b.build();
        assert_eq!(reg.default_entry().unwrap().source.name(), "raw");
    }

    #[test]
    fn default_entry_falls_back_to_first_registered() {
        let mut b = SearchRegistry::builder();
        b.register(entry("eng", false)).unwrap();
        b.register(entry("raw", false)).unwrap();
        let reg = b.build();
        assert_eq!(reg.default_entry().unwrap().source.name(), "eng");
    }

    #[test]
    fn default_entry_is_none_on_empty_registry() {
        let reg = SearchRegistry::builder().build();
        assert!(reg.default_entry().is_none());
        assert!(reg.is_empty());
    }

    #[test]
    fn iter_preserves_registration_order() {
        let mut b = SearchRegistry::builder();
        b.register(entry("b", false)).unwrap();
        b.register(entry("a", false)).unwrap();
        let reg = b.build();
        let names: Vec<&str> = reg.iter().map(|e| e.source.name()).collect();
        assert_eq!(names, vec!["b", "a"]);
    }
}
