//! `tsundoku refresh-provider-cache [--provider id]`.
//!
//! Iterates the metadata registry (or one named provider) and calls
//! `refresh_cache()` on each. Providers without an offline cache report
//! `RefreshStatus::NotSupported` and are skipped silently.
//!
//! Successful refreshes are appended to the `provider_cache_state` table
//! by way of [`td_db::repos::provider_cache_state_repo::append`] so the
//! API and UI can show "when was the cache last refreshed". Distinct
//! from `tsundoku refresh-series`, which walks the existing series rows
//! and re-fetches each one's metadata from the active provider.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use td_db::repos::provider_cache_state_repo;
use td_metadata::{MetadataProvider, RefreshStatus, RefreshSummary};

pub async fn run(config_path: PathBuf, provider_id: Option<String>) -> anyhow::Result<()> {
    let cfg = td_config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    super::init_tracing(&cfg);

    let db = td_db::connect(&cfg).await?;
    td_db::run_migrations(&db).await?;
    let limiter = crate::http_limiter::build(&cfg.ingestion.http);
    let registry = crate::metadata::build_registry(&cfg, limiter).await?;

    let providers: Vec<&Arc<dyn MetadataProvider>> = match provider_id.as_deref() {
        Some(id) => {
            let provider = registry
                .get(id)
                .ok_or_else(|| anyhow::anyhow!("provider {id:?} is not registered"))?;
            vec![provider]
        }
        None => registry.iter().map(|(_, p)| p).collect(),
    };

    let mut summaries = Vec::with_capacity(providers.len());
    for provider in providers {
        let summary = provider.refresh_cache().await?;
        match &summary.status {
            RefreshStatus::Refreshed { records, version } => {
                tracing::info!(
                    provider = provider.id(),
                    records,
                    cache_version = ?version,
                    bytes = ?summary.bytes_downloaded,
                    "cache refreshed"
                );
                let started = summary.finished_at.timestamp();
                provider_cache_state_repo::append(
                    &db,
                    provider.id(),
                    started,
                    version.as_deref(),
                    Some(*records as i64),
                    None,
                    summary.bytes_downloaded.map(|b| b as i64),
                )
                .await?;
            }
            RefreshStatus::UpToDate => {
                tracing::info!(
                    provider = provider.id(),
                    "cache up to date; no refresh needed"
                );
            }
            RefreshStatus::NotSupported => {
                tracing::info!(
                    provider = provider.id(),
                    "provider does not maintain an offline cache; skipping"
                );
            }
            RefreshStatus::Skipped { message } => {
                tracing::warn!(provider = provider.id(), %message, "cache refresh skipped");
            }
        }
        summaries.push(summary);
    }

    render_summary(&summaries);
    Ok(())
}

fn render_summary(summaries: &[RefreshSummary]) {
    println!("\nrefresh-provider-cache summary:");
    for s in summaries {
        let status = match &s.status {
            RefreshStatus::Refreshed { records, version } => {
                format!(
                    "refreshed ({records} records{})",
                    version
                        .as_deref()
                        .map(|v| format!(", version={v}"))
                        .unwrap_or_default()
                )
            }
            RefreshStatus::UpToDate => "up to date".into(),
            RefreshStatus::NotSupported => "not supported (no offline cache)".into(),
            RefreshStatus::Skipped { message } => format!("skipped: {message}"),
        };
        println!("  {}  →  {status}", s.provider);
    }
}
