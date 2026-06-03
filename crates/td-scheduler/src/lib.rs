//! Cron-driven scheduler for tsundoku.
//!
//! Wraps `tokio_cron_scheduler::JobScheduler` and registers one job per
//! enabled discovery source (with a cron) and one job per metadata provider
//! that has a configured refresh cron. Sources whose cron is `None` or whose
//! `enabled = false` are skipped; providers without a configured refresh
//! cron are skipped too.
//!
//! The trait-level work — polling a source, resolving the resulting
//! releases, refreshing a provider's cache — lives in the [`jobs`] module.
//! `Scheduler` itself only handles construction, registration, and
//! lifecycle.
//!
//! Concurrency: each registered job acquires a per-key
//! [`tokio::sync::Mutex`] from [`JobLocks`] before doing any work and
//! `try_lock`s rather than blocks. This means an overlapping tick is
//! silently dropped (logged at debug level) rather than queued behind the
//! previous run. The single-writer SQLite pool makes serialisation
//! mandatory at the DB layer anyway; this just makes the intent explicit at
//! the scheduler boundary.

pub mod dispatch;
pub mod error_kind;
pub mod events;
pub mod jobs;

pub use dispatch::try_dispatch;
pub use events::{JOB_EVENT_BUFFER, JobEvent, JobKind, JobPhase, JobProgress, JobResult};

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use dashmap::DashMap;
use sea_orm::DatabaseConnection;
use td_config::{AppConfig, IngestionConfig};
use td_metadata::MetadataRegistry;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;
use td_source::SourceRegistry;
use tokio::sync::{Mutex, broadcast};
use tokio_cron_scheduler::JobScheduler;

/// Per-key in-flight markers. Cloned into every job closure so overlapping
/// ticks for the same source / provider are skipped rather than queued.
#[derive(Default)]
pub struct JobLocks {
    sources: DashMap<String, Arc<Mutex<()>>>,
    providers: DashMap<String, Arc<Mutex<()>>>,
    /// Per-provider lock for the bulk series-metadata refresh job. Kept
    /// separate from `providers` so an ongoing cache refresh doesn't gate
    /// a series-refresh tick and vice versa (the two jobs touch different
    /// state).
    series_refreshes: DashMap<String, Arc<Mutex<()>>>,
    /// Single global lock for the review-queue retry-all operation. Held
    /// for the lifetime of one batch so a second click is reported as
    /// skipped rather than spawning a parallel walk over the same rows.
    retry_all_releases: Arc<Mutex<()>>,
    /// Single global lock for the Codex presence sync. One sweep at a time;
    /// the cron tick and the manual `POST /codex/refresh` both `try_lock` it,
    /// so a manual kick during a scheduled sweep is reported as skipped.
    codex_sync: Arc<Mutex<()>>,
}

impl JobLocks {
    pub fn source_lock(&self, name: &str) -> Arc<Mutex<()>> {
        self.sources
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub fn provider_lock(&self, id: &str) -> Arc<Mutex<()>> {
        self.providers
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub fn series_refresh_lock(&self, provider_id: &str) -> Arc<Mutex<()>> {
        self.series_refreshes
            .entry(provider_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub fn retry_all_releases_lock(&self) -> Arc<Mutex<()>> {
        self.retry_all_releases.clone()
    }

    pub fn codex_sync_lock(&self) -> Arc<Mutex<()>> {
        self.codex_sync.clone()
    }
}

/// Shared state every job needs. Cloned (cheap `Arc` bumps) into each job
/// closure at registration time.
#[derive(Clone)]
pub struct SchedulerContext {
    pub db: DatabaseConnection,
    pub sources: Arc<SourceRegistry>,
    pub metadata: Arc<MetadataRegistry>,
    pub ingestion: IngestionConfig,
    pub locks: Arc<JobLocks>,
    /// Title cleaner shared across every poll-tick resolver. Built once
    /// at startup from the built-in keyword list plus
    /// `ingestion.cleanup.extra_format_keywords`.
    pub query_builder: Arc<QueryBuilder>,
    /// Optional MangaUpdates redirect resolver. When configured, the
    /// resolver translates legacy `series.html?id=NNN` IDs into modern
    /// alphanumeric slugs before the foreign-id lookup step. `None` in
    /// tests that don't need network access.
    pub mangaupdates_redirector: Option<Arc<MangaUpdatesRedirector>>,
    /// Broadcast channel for job-lifecycle / progress events. Shared with
    /// `td-api::AppState::job_events` so cron-driven progress frames and
    /// manual-trigger lifecycle frames fan out through one stream.
    pub job_events: broadcast::Sender<JobEvent>,
    /// Codex client for the presence sync. `None` when `[codex] enabled =
    /// false`; the sync cron is then not registered. Shared with
    /// `td-api::AppState` so the manual `POST /codex/refresh` trigger drives
    /// the same client under the same lock.
    pub codex_client: Option<Arc<td_codex::CodexClient>>,
    /// Torrent-download client for the background connection re-test. `None`
    /// when `[download] enabled = false`; the health cron is then not
    /// registered. Shared with `td-api::AppState` so the cron and the manual
    /// `POST /download/test` drive the same client.
    pub download_client: Option<Arc<dyn td_download::DownloadClient>>,
}

/// Owns the running scheduler. Drop or call [`Self::shutdown`] to stop.
pub struct Scheduler {
    inner: JobScheduler,
    registered_jobs: usize,
}

impl Scheduler {
    /// Build a scheduler from the application config + already-built
    /// registries. Reads `cfg.sources[*].cron` and
    /// `cfg.providers.<id>.offline_refresh_cron`, registering one job for
    /// every entry that has both an enabled flag (where applicable) and a
    /// cron expression.
    pub async fn build(cfg: &AppConfig, ctx: SchedulerContext) -> Result<Self> {
        let inner = JobScheduler::new()
            .await
            .map_err(|e| anyhow!("creating tokio-cron-scheduler: {e}"))?;
        let mut registered = 0usize;

        // Source jobs.
        for src in &cfg.sources {
            if !src.enabled {
                continue;
            }
            let Some(cron) = src.cron.as_deref().filter(|s| !s.is_empty()) else {
                tracing::info!(source = %src.name, "no cron configured; skipping scheduled poll");
                continue;
            };
            let Some(source) = ctx.sources.get(&src.name).cloned() else {
                tracing::warn!(
                    source = %src.name,
                    "source has a cron but is not in the registry (disabled or unknown kind); skipping"
                );
                continue;
            };
            let normalized = normalize_cron(cron)
                .with_context(|| format!("normalising cron for source {:?}", src.name))?;
            let job = jobs::poll_source::build(
                &normalized,
                source,
                ctx.db.clone(),
                ctx.metadata.clone(),
                ctx.ingestion.clone(),
                ctx.locks.clone(),
                ctx.query_builder.clone(),
                ctx.mangaupdates_redirector.clone(),
                ctx.job_events.clone(),
            )?;
            inner
                .add(job)
                .await
                .map_err(|e| anyhow!("registering poll job for source {:?}: {e}", src.name))?;
            tracing::info!(source = %src.name, cron = %normalized, "registered scheduled poll job");
            registered += 1;
        }

        // Provider cache-refresh jobs. The provider must be both registered
        // and have a non-empty cron string. v1 only knows about mangabaka;
        // future providers wire here in the same shape.
        if let Some(cron) = cfg
            .providers
            .mangabaka
            .offline_refresh_cron
            .as_deref()
            .filter(|s| !s.is_empty())
        {
            register_provider_job(&inner, &ctx, "mangabaka", cron, &mut registered).await?;
        }

        // Bulk series-metadata refresh job. Always against the *active*
        // provider; non-active providers in the registry are used only for
        // cross-provider foreign-ID resolution and review-UI search.
        if let Some(cron) = cfg
            .metadata
            .series_refresh
            .cron
            .as_deref()
            .filter(|s| !s.is_empty())
        {
            register_series_refresh_job(
                &inner,
                &ctx,
                &cfg.metadata.active_provider,
                cron,
                cfg.metadata.series_refresh.batch_size,
                cfg.metadata.series_refresh.min_age_days,
                &mut registered,
            )
            .await?;
        }

        // Codex presence sync. Registered only when the integration is
        // enabled, the client was built, and a sync_cron is set. The manual
        // `POST /codex/refresh` trigger works regardless of the cron.
        if cfg.codex.enabled
            && let Some(client) = ctx.codex_client.clone()
            && let Some(cron) = cfg.codex.sync_cron.as_deref().filter(|s| !s.is_empty())
        {
            let normalized = normalize_cron(cron).context("normalising codex sync_cron")?;
            let job = jobs::sync_codex::build(
                &normalized,
                client,
                ctx.db.clone(),
                ctx.locks.clone(),
                ctx.job_events.clone(),
            )?;
            inner
                .add(job)
                .await
                .map_err(|e| anyhow!("registering codex sync job: {e}"))?;
            tracing::info!(cron = %normalized, "registered codex presence sync job");
            registered += 1;
        }

        // Download-client health re-test. Registered only when the integration
        // is enabled, the client was built, and a health_cron is set. The
        // manual `POST /download/test` works regardless of the cron.
        if cfg.download.enabled
            && let Some(client) = ctx.download_client.clone()
            && let Some(cron) = cfg
                .download
                .health_cron
                .as_deref()
                .filter(|s| !s.is_empty())
        {
            let normalized = normalize_cron(cron).context("normalising download health_cron")?;
            let job = jobs::test_download::build(&normalized, client, ctx.db.clone())?;
            inner
                .add(job)
                .await
                .map_err(|e| anyhow!("registering download health job: {e}"))?;
            tracing::info!(cron = %normalized, "registered download health job");
            registered += 1;
        }

        // Hourly review-queue snapshot. Fires at minute 5 to stagger off
        // both the typical "0 * * * *" provider crons and the more
        // frequent source ticks. Operators don't get a config knob for
        // this today — the schedule is fixed.
        let snapshot_cron =
            normalize_cron("5 * * * *").context("normalising review-queue snapshot cron")?;
        let snapshot_job = jobs::snapshot_review_queue::build(&snapshot_cron, ctx.db.clone())?;
        inner
            .add(snapshot_job)
            .await
            .map_err(|e| anyhow!("registering review-queue snapshot job: {e}"))?;
        tracing::info!(cron = %snapshot_cron, "registered review-queue snapshot job");
        registered += 1;

        Ok(Self {
            inner,
            registered_jobs: registered,
        })
    }

    /// Start the scheduler's internal task loop. After this returns, jobs
    /// fire on their configured cadence until [`Self::shutdown`] is called
    /// or the process exits.
    pub async fn start(&self) -> Result<()> {
        self.inner
            .start()
            .await
            .map_err(|e| anyhow!("starting scheduler: {e}"))?;
        tracing::info!(jobs = self.registered_jobs, "scheduler started");
        Ok(())
    }

    /// Stop the scheduler and any in-flight job ticks. Best-effort: errors
    /// from the underlying library are logged and swallowed.
    pub async fn shutdown(&mut self) -> Result<()> {
        if let Err(e) = self.inner.shutdown().await {
            tracing::warn!(error = ?e, "scheduler shutdown returned an error");
        }
        Ok(())
    }

    /// Number of jobs successfully registered. Useful for tests and the
    /// startup banner.
    pub fn job_count(&self) -> usize {
        self.registered_jobs
    }
}

async fn register_provider_job(
    inner: &JobScheduler,
    ctx: &SchedulerContext,
    provider_id: &str,
    cron: &str,
    registered: &mut usize,
) -> Result<()> {
    let Some(provider) = ctx.metadata.get(provider_id).cloned() else {
        tracing::warn!(
            provider = %provider_id,
            "refresh cron set but provider is not registered; skipping"
        );
        return Ok(());
    };
    let normalized = normalize_cron(cron)
        .with_context(|| format!("normalising refresh cron for provider {provider_id:?}"))?;
    let job = jobs::refresh_provider_cache::build(
        &normalized,
        provider,
        ctx.db.clone(),
        ctx.locks.clone(),
        ctx.job_events.clone(),
    )?;
    inner
        .add(job)
        .await
        .map_err(|e| anyhow!("registering refresh job for provider {provider_id:?}: {e}"))?;
    tracing::info!(
        provider = %provider_id,
        cron = %normalized,
        "registered scheduled provider refresh job"
    );
    *registered += 1;
    Ok(())
}

async fn register_series_refresh_job(
    inner: &JobScheduler,
    ctx: &SchedulerContext,
    active_provider_id: &str,
    cron: &str,
    batch_size: u32,
    min_age_days: u32,
    registered: &mut usize,
) -> Result<()> {
    let Some(provider) = ctx.metadata.get(active_provider_id).cloned() else {
        tracing::warn!(
            provider = %active_provider_id,
            "series_refresh cron set but active provider is not registered; skipping"
        );
        return Ok(());
    };
    let normalized = normalize_cron(cron).with_context(|| {
        format!("normalising series_refresh cron for provider {active_provider_id:?}")
    })?;
    let min_age_seconds = (min_age_days as i64).saturating_mul(86_400);
    let job = jobs::refresh_series_metadata::build(
        &normalized,
        provider,
        ctx.db.clone(),
        ctx.locks.clone(),
        batch_size,
        min_age_seconds,
        ctx.job_events.clone(),
    )?;
    inner.add(job).await.map_err(|e| {
        anyhow!("registering series_refresh job for provider {active_provider_id:?}: {e}")
    })?;
    tracing::info!(
        provider = %active_provider_id,
        cron = %normalized,
        batch_size,
        min_age_days,
        "registered scheduled series-metadata refresh job"
    );
    *registered += 1;
    Ok(())
}

/// Accept either 5-field (`m h dom mon dow`) or 6-/7-field cron expressions.
/// tokio-cron-scheduler delegates to the `cron` crate, which requires at
/// least 6 fields (seconds-included). Five-field strings are normalised by
/// prepending `0 ` so the familiar standard-cron format keeps working in
/// config files.
pub(crate) fn normalize_cron(expr: &str) -> Result<String> {
    let trimmed = expr.trim();
    let fields = trimmed.split_whitespace().count();
    match fields {
        5 => Ok(format!("0 {trimmed}")),
        6 | 7 => Ok(trimmed.to_string()),
        n => Err(anyhow!(
            "cron expression must have 5, 6, or 7 fields, got {n}: {expr:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_field_cron_gets_zero_second_prefix() {
        assert_eq!(normalize_cron("*/30 * * * *").unwrap(), "0 */30 * * * *");
        assert_eq!(normalize_cron("0 4 * * 0").unwrap(), "0 0 4 * * 0");
    }

    #[test]
    fn six_field_cron_passes_through() {
        assert_eq!(normalize_cron("*/5 * * * * *").unwrap(), "*/5 * * * * *");
    }

    #[test]
    fn seven_field_cron_passes_through() {
        assert_eq!(
            normalize_cron("0 */5 * * * * 2026").unwrap(),
            "0 */5 * * * * 2026"
        );
    }

    #[test]
    fn malformed_cron_is_rejected_early() {
        let err = normalize_cron("0 0 0 0").expect_err("4-field crons are unsupported");
        let msg = err.to_string();
        assert!(msg.contains("must have 5, 6, or 7 fields"), "{msg}");
    }
}
