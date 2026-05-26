//! Metadata-registry construction.
//!
//! Single entry point used by every CLI command that needs a registry:
//! `serve`, `migrate`, `refresh-metadata`, and (future) `poll` / `resolve`.
//! Reads `[providers.*]` blocks from config and the active provider id from
//! `[metadata]`, builds a [`td_metadata::MetadataRegistry`], and returns it.
//!
//! Adding a new provider means importing its crate and adding one
//! `register` call here. No other crates change.

use std::sync::Arc;

use anyhow::Context;
use td_config::AppConfig;
use td_metadata::{MetadataProvider, MetadataRegistry};
use td_metadata_mangabaka::MangabakaProvider;

pub async fn build_registry(cfg: &AppConfig) -> anyhow::Result<MetadataRegistry> {
    let mut builder = MetadataRegistry::builder();
    let paths = cfg.storage.paths();
    paths.ensure()?;

    if cfg.providers.mangabaka.enabled {
        let cache_dir = paths.provider_cache_dir_for("mangabaka");
        let provider: Arc<dyn MetadataProvider> =
            Arc::new(MangabakaProvider::from_config(&cfg.providers.mangabaka, cache_dir).await?);
        builder
            .register(provider)
            .context("registering mangabaka provider")?;
    }
    // Future: anilist, mal, mangaupdates, mangadex via the same pattern.

    builder.set_active(&cfg.metadata.active_provider);
    builder.build().context("building metadata registry")
}
