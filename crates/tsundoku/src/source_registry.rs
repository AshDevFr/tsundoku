//! Discovery-source registry construction.
//!
//! Single entry point used by every CLI command that needs the registry:
//! `serve`, `poll`, and (future) the cron scheduler. Reads the
//! `[[sources]]` array from config and dispatches each entry to its
//! kind-specific constructor.
//!
//! Adding a new source kind = importing its crate and adding one arm to
//! the `match`. No other crates change.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow};
use td_config::{AppConfig, SourceConfig};
use td_http::HttpLimiter;
use td_source::{DiscoverySource, SourceRegistry};
use td_source_nyaa::{NyaaSource, NyaaSourceConfig};

pub fn build_registry(
    cfg: &AppConfig,
    limiter: Arc<HttpLimiter>,
) -> anyhow::Result<SourceRegistry> {
    let mut builder = SourceRegistry::builder();
    for src in &cfg.sources {
        if !src.enabled {
            tracing::info!(source = %src.name, "skipping disabled source");
            continue;
        }
        let provider: Arc<dyn DiscoverySource> = construct_source(src, limiter.clone())?;
        builder
            .register(provider)
            .with_context(|| format!("registering source {:?}", src.name))?;
    }
    Ok(builder.build())
}

fn construct_source(
    src: &SourceConfig,
    limiter: Arc<HttpLimiter>,
) -> anyhow::Result<Arc<dyn DiscoverySource>> {
    match src.kind.as_str() {
        td_source_nyaa::SOURCE_KIND => {
            let opts = src.nyaa.as_ref().ok_or_else(|| {
                anyhow!(
                    "source {:?} (kind=nyaa) is missing the [sources.nyaa] options block",
                    src.name
                )
            })?;
            if opts.feed_url.is_empty() {
                return Err(anyhow!(
                    "source {:?} (kind=nyaa) requires [sources.nyaa].feed_url",
                    src.name
                ));
            }
            let nyaa_cfg = NyaaSourceConfig {
                name: src.name.clone(),
                feed_url: opts.feed_url.clone(),
                timeout: Duration::from_secs(opts.timeout_seconds.max(1) as u64),
                fetch_details: opts.fetch_details,
                site_base_url: opts.site_base_url.clone(),
                max_pages: opts.max_pages.max(1),
            };
            let nyaa = NyaaSource::from_config(nyaa_cfg, limiter)
                .with_context(|| format!("building nyaa source {:?}", src.name))?;
            Ok(Arc::new(nyaa))
        }
        other => Err(anyhow!(
            "source {:?} has unknown kind {other:?} (only \"nyaa\" is supported in v1)",
            src.name
        )),
    }
}
