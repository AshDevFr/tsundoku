//! In-process registry of [`MetadataProvider`] implementations.
//!
//! Built once at startup from config, then held as `Arc<MetadataRegistry>`
//! across the axum app state, the scheduler, and the CLI. The registry has
//! exactly one *active* provider (drives auto-detect resolution); other
//! registered providers exist for the review UI's manual search and for
//! foreign-ID chains.

use std::collections::HashMap;
use std::sync::Arc;

use crate::provider::MetadataProvider;

#[derive(thiserror::Error, Debug)]
pub enum RegistryError {
    #[error("metadata.active_provider = {0:?} but no provider with that id is registered")]
    UnknownActive(String),
    #[error("no providers registered; at least one is required")]
    Empty,
    #[error("provider id {0:?} registered more than once")]
    Duplicate(String),
}

pub struct MetadataRegistry {
    providers: HashMap<String, Arc<dyn MetadataProvider>>,
    active_id: String,
}

impl MetadataRegistry {
    pub fn builder() -> MetadataRegistryBuilder {
        MetadataRegistryBuilder::default()
    }

    /// The provider designated to run the auto-detect resolution path.
    pub fn active(&self) -> &Arc<dyn MetadataProvider> {
        // Invariant: `active_id` is checked at build time.
        self.providers
            .get(&self.active_id)
            .expect("active provider id must point at a registered provider")
    }

    pub fn active_id(&self) -> &str {
        &self.active_id
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn MetadataProvider>> {
        self.providers.get(id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.providers.contains_key(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Arc<dyn MetadataProvider>)> {
        self.providers.iter().map(|(id, p)| (id.as_str(), p))
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.providers.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

#[derive(Default)]
pub struct MetadataRegistryBuilder {
    providers: HashMap<String, Arc<dyn MetadataProvider>>,
    active_id: Option<String>,
}

impl MetadataRegistryBuilder {
    pub fn register(
        &mut self,
        provider: Arc<dyn MetadataProvider>,
    ) -> Result<&mut Self, RegistryError> {
        let id = provider.id().to_string();
        if self.providers.contains_key(&id) {
            return Err(RegistryError::Duplicate(id));
        }
        self.providers.insert(id, provider);
        Ok(self)
    }

    pub fn set_active(&mut self, id: impl Into<String>) -> &mut Self {
        self.active_id = Some(id.into());
        self
    }

    pub fn build(self) -> Result<MetadataRegistry, RegistryError> {
        if self.providers.is_empty() {
            return Err(RegistryError::Empty);
        }
        let active_id = self
            .active_id
            .ok_or_else(|| RegistryError::UnknownActive(String::new()))?;
        if !self.providers.contains_key(&active_id) {
            return Err(RegistryError::UnknownActive(active_id));
        }
        Ok(MetadataRegistry {
            providers: self.providers,
            active_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::MetadataResult;
    use crate::types::{SearchHit, SeriesMetadata};
    use async_trait::async_trait;

    /// Test double that records nothing useful but satisfies the trait.
    /// Demonstrates that the default `resolve_by_foreign_id` and
    /// `refresh_cache` impls compose cleanly without an override.
    struct StubProvider {
        id: &'static str,
    }

    #[async_trait]
    impl MetadataProvider for StubProvider {
        fn id(&self) -> &str {
            self.id
        }
        fn display_name(&self) -> &str {
            "Stub"
        }
        async fn get(&self, _external_id: &str) -> MetadataResult<Option<SeriesMetadata>> {
            Ok(None)
        }
        async fn search(&self, _query: &str, _limit: u32) -> MetadataResult<Vec<SearchHit>> {
            Ok(Vec::new())
        }
    }

    fn stub(id: &'static str) -> Arc<dyn MetadataProvider> {
        Arc::new(StubProvider { id })
    }

    #[test]
    fn builder_requires_at_least_one_provider() {
        let mut b = MetadataRegistry::builder();
        b.set_active("mangabaka");
        match b.build() {
            Err(RegistryError::Empty) => {}
            Err(other) => panic!("expected RegistryError::Empty, got {other:?}"),
            Ok(_) => panic!("expected build to fail with Empty"),
        }
    }

    #[test]
    fn builder_rejects_unknown_active_id() {
        let mut b = MetadataRegistry::builder();
        b.register(stub("mangabaka")).unwrap();
        b.set_active("anilist");
        match b.build() {
            Err(RegistryError::UnknownActive(s)) if s == "anilist" => {}
            Err(other) => panic!("expected UnknownActive(\"anilist\"), got {other:?}"),
            Ok(_) => panic!("expected build to fail with UnknownActive"),
        }
    }

    #[test]
    fn builder_rejects_duplicate_provider_id() {
        let mut b = MetadataRegistry::builder();
        b.register(stub("mangabaka")).unwrap();
        match b.register(stub("mangabaka")) {
            Err(RegistryError::Duplicate(s)) if s == "mangabaka" => {}
            Err(other) => panic!("expected Duplicate(\"mangabaka\"), got {other:?}"),
            Ok(_) => panic!("expected duplicate to fail"),
        }
    }

    #[test]
    fn builder_produces_registry_with_active_pointer() {
        let mut b = MetadataRegistry::builder();
        b.register(stub("mangabaka")).unwrap();
        b.register(stub("anilist")).unwrap();
        b.set_active("anilist");
        let reg = b.build().expect("registry should build");
        assert_eq!(reg.active_id(), "anilist");
        assert_eq!(reg.active().id(), "anilist");
        assert!(reg.contains("mangabaka"));
        assert!(reg.contains("anilist"));
        assert!(!reg.contains("mal"));
        assert_eq!(reg.len(), 2);
        let mut ids: Vec<&str> = reg.ids().collect();
        ids.sort();
        assert_eq!(ids, vec!["anilist", "mangabaka"]);
    }

    #[tokio::test]
    async fn default_resolve_by_foreign_id_returns_none() {
        let p = StubProvider { id: "s" };
        let got = p.resolve_by_foreign_id("mangaupdates", "1").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn default_refresh_cache_reports_not_supported() {
        let p = StubProvider { id: "s" };
        let summary = p.refresh_cache().await.unwrap();
        assert_eq!(summary.provider, "s");
        assert!(matches!(
            summary.status,
            crate::types::RefreshStatus::NotSupported
        ));
    }
}
