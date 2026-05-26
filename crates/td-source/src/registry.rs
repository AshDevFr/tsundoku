//! In-process registry of [`crate::DiscoverySource`] instances.
//!
//! Built once at startup from config (one entry per `[[sources]]` block),
//! then held as `Arc<SourceRegistry>` across the scheduler, the API handlers
//! (manual-poll endpoint), and the `tsundoku poll` CLI. Sources do not have
//! an active/non-active distinction: every registered source is polled on
//! its own cron schedule.

use std::collections::HashMap;
use std::sync::Arc;

use crate::source::DiscoverySource;

#[derive(thiserror::Error, Debug)]
pub enum RegistryError {
    #[error("source named {0:?} registered more than once")]
    DuplicateName(String),
}

pub struct SourceRegistry {
    sources: HashMap<String, Arc<dyn DiscoverySource>>,
}

impl SourceRegistry {
    pub fn builder() -> SourceRegistryBuilder {
        SourceRegistryBuilder::default()
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn DiscoverySource>> {
        self.sources.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Arc<dyn DiscoverySource>)> {
        self.sources.iter().map(|(name, s)| (name.as_str(), s))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.sources.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

#[derive(Default)]
pub struct SourceRegistryBuilder {
    sources: HashMap<String, Arc<dyn DiscoverySource>>,
}

impl SourceRegistryBuilder {
    /// Register a source. Source `name`s must be unique across the whole
    /// registry (not just per-kind) because the manual-poll endpoint keys
    /// on name alone.
    pub fn register(
        &mut self,
        source: Arc<dyn DiscoverySource>,
    ) -> Result<&mut Self, RegistryError> {
        let name = source.name().to_string();
        if self.sources.contains_key(&name) {
            return Err(RegistryError::DuplicateName(name));
        }
        self.sources.insert(name, source);
        Ok(self)
    }

    pub fn build(self) -> SourceRegistry {
        SourceRegistry {
            sources: self.sources,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SourceResult;
    use crate::release::{PollContext, PollOutcome};
    use async_trait::async_trait;

    struct StubSource {
        name: &'static str,
        kind: &'static str,
    }

    #[async_trait]
    impl DiscoverySource for StubSource {
        fn name(&self) -> &str {
            self.name
        }
        fn kind(&self) -> &str {
            self.kind
        }
        async fn poll(&self, _ctx: &PollContext) -> SourceResult<PollOutcome> {
            Ok(PollOutcome::default())
        }
    }

    fn stub(name: &'static str, kind: &'static str) -> Arc<dyn DiscoverySource> {
        Arc::new(StubSource { name, kind })
    }

    #[test]
    fn registers_two_distinct_sources_of_the_same_kind() {
        let mut b = SourceRegistry::builder();
        b.register(stub("trusted", "nyaa")).unwrap();
        b.register(stub("test-feed", "nyaa")).unwrap();
        let reg = b.build();
        assert_eq!(reg.len(), 2);
        assert!(reg.get("trusted").is_some());
        assert!(reg.get("test-feed").is_some());
        assert!(reg.get("does-not-exist").is_none());
    }

    #[test]
    fn rejects_duplicate_name_even_across_kinds() {
        let mut b = SourceRegistry::builder();
        b.register(stub("trusted", "nyaa")).unwrap();
        let result = b.register(stub("trusted", "some-other-kind"));
        match result {
            Err(RegistryError::DuplicateName(s)) if s == "trusted" => {}
            Err(other) => panic!("expected DuplicateName(\"trusted\"), got {other:?}"),
            Ok(_) => panic!("expected duplicate-name registration to fail"),
        }
    }

    #[test]
    fn builds_empty_registry_without_error() {
        let reg = SourceRegistry::builder().build();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert_eq!(reg.names().count(), 0);
    }
}
