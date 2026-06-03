use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use sea_orm::DatabaseConnection;
use td_api::AppState;
use td_db::repos::run_metrics_repo::trigger;
use td_metadata::MetadataRegistry;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;
use td_scheduler::{JobLocks, Scheduler, SchedulerContext, jobs::refresh_provider_cache};

pub async fn run(config_path: PathBuf, explicit_config: bool) -> anyhow::Result<()> {
    // Default-path bootstrap: if the operator ran `serve` without `--config`
    // and the default file is missing, drop a commented starter template
    // there so the first run is usable instead of failing on a missing
    // config or booting with bare defaults that can't poll anything.
    // An explicit `--config <path>` is treated as a load-or-fail intent.
    if !config_path.exists() {
        if explicit_config {
            // Explicit `--config` is a "load this exact file" directive.
            // Silently falling back to figment defaults would mask a typo
            // or a missing mount and produce a binary that boots with no
            // sources and no admin_token. Fail loudly instead.
            anyhow::bail!(
                "config file not found at {} (passed via --config); create the file or omit --config to use the default path",
                config_path.display()
            );
        }
        eprintln!(
            "config file not found at {}; writing a starter template (edit it before exposing the API)",
            config_path.display()
        );
        td_config::write_starter(&config_path, false)?;
    }

    let cfg = td_config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    super::init_tracing(&cfg);

    let db = td_db::connect(&cfg).await?;
    td_db::run_migrations(&db).await?;

    // Build the process-wide outbound-HTTP limiter first; every component
    // that makes external requests routes through it so per-host
    // serialization and min-gap are enforced uniformly.
    let limiter = crate::http_limiter::build(&cfg.ingestion.http);

    // Build the Codex client once (when the integration is enabled) and share
    // it between the startup probe, the sync cron, and the manual refresh
    // endpoint so they all hit Codex through the same limited client.
    let codex_client = build_codex_client(&cfg.codex, limiter.clone());

    // Build the torrent-download client once (when the integration is enabled)
    // so the send endpoint shares the same limited client.
    let download_client = build_download_client(&cfg.download, limiter.clone());

    // Probe the torrent client once at startup so the admin Download page shows
    // reachable/unreachable immediately instead of "never tested". Non-fatal.
    spawn_download_startup_probe(download_client.clone(), db.clone());

    // Probe Codex's public /info endpoint once at startup so the operator sees
    // the connected name/version (or a warning) immediately. Non-fatal: a
    // Codex that is down at boot must not block tsundoku from serving; the sync
    // cron retries.
    spawn_codex_startup_probe(
        codex_client.clone(),
        db.clone(),
        cfg.codex.normalized_base_url().unwrap_or_default(),
    );

    let sources = Arc::new(crate::source_registry::build_registry(
        &cfg,
        limiter.clone(),
    )?);
    let metadata = Arc::new(crate::metadata::build_registry(&cfg, limiter.clone()).await?);
    let locks = Arc::new(JobLocks::default());
    // One shared MangaUpdates redirector for the whole process: scheduler
    // ticks and the API retry handler both go through the same throttle.
    let user_agent = concat!(
        "tsundoku/",
        env!("CARGO_PKG_VERSION"),
        " (+https://github.com/AshDevFr/tsundoku)"
    );
    let mu_redirector = match MangaUpdatesRedirector::new(user_agent, limiter.clone()) {
        Ok(r) => Some(Arc::new(r)),
        Err(e) => {
            tracing::warn!(error = ?e, "failed to build mangaupdates redirector; legacy MU URLs will be dropped");
            None
        }
    };
    // Build the title cleaner once: built-in keyword list + operator
    // extras from `[ingestion.cleanup]`. Invalid extras (regex
    // metacharacters, empty strings) make the binary refuse to start.
    let query_builder = Arc::new(
        QueryBuilder::new(&cfg.ingestion.cleanup.extra_format_keywords)
            .context("building title cleaner from ingestion.cleanup config")?,
    );

    // Single broadcast channel feeds both the scheduler (cron-driven
    // progress frames) and the API (manual-trigger lifecycle + SSE
    // delivery). The sender clones cheaply; one underlying ring buffer.
    let (job_events, _) = tokio::sync::broadcast::channel(td_api::JOB_EVENT_BUFFER);

    let ctx = SchedulerContext {
        db: db.clone(),
        sources: sources.clone(),
        metadata: metadata.clone(),
        ingestion: cfg.ingestion.clone(),
        locks: locks.clone(),
        query_builder: query_builder.clone(),
        mangaupdates_redirector: mu_redirector.clone(),
        job_events: job_events.clone(),
        codex_client: codex_client.clone(),
        download_client: download_client.clone(),
    };
    let scheduler = Scheduler::build(&cfg, ctx).await?;
    scheduler.start().await?;

    // If any provider has an offline cache configured but no dump on disk
    // yet, kick off an immediate refresh in the background so the operator
    // does not have to wait for the next cron tick. The job goes through the
    // same locked code path as the scheduler tick, so a concurrently firing
    // cron just `try_lock`s and skips. Errors are logged inside `run_tick`.
    spawn_startup_refreshes(
        metadata.clone(),
        db.clone(),
        locks.clone(),
        job_events.clone(),
    );

    let state = AppState {
        db,
        sources,
        metadata,
        ingestion: cfg.ingestion.clone(),
        auth: Arc::new(cfg.auth.clone()),
        locks,
        sources_config: Arc::new(cfg.sources.clone()),
        providers_config: Arc::new(cfg.providers.clone()),
        metadata_config: Arc::new(cfg.metadata.clone()),
        query_builder,
        mangaupdates_redirector: mu_redirector,
        job_events,
        cover_cache_dir: Some(cfg.storage.paths().cover_cache_dir),
        codex: Arc::new(cfg.codex.clone()),
        codex_client,
        download: Arc::new(cfg.download.clone()),
        download_client,
    };
    let app = td_api::router(state, &cfg);

    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "listening");
    axum::serve(listener, app).await?;

    // axum::serve returns when the listener is closed (Ctrl-C on most
    // platforms). Drop the scheduler explicitly so any in-flight tick is
    // cancelled before we exit.
    drop(scheduler);
    Ok(())
}

/// Build the shared Codex client when the integration is enabled. Returns
/// `None` when disabled, when the required fields are missing (config
/// validation normally prevents this), or when the reqwest client fails to
/// build (logged). The same client is shared by the startup probe, the sync
/// cron, and the manual refresh endpoint.
fn build_codex_client(
    codex: &td_config::CodexConfig,
    limiter: Arc<td_http::HttpLimiter>,
) -> Option<Arc<td_codex::CodexClient>> {
    if !codex.enabled {
        return None;
    }
    let (Some(base_url), Some(api_key)) = (codex.normalized_base_url(), codex.api_key.clone())
    else {
        tracing::warn!("codex.enabled is true but base_url/api_key is missing; codex disabled");
        return None;
    };
    let timeout = std::time::Duration::from_secs(codex.timeout_seconds as u64);
    match td_codex::CodexClient::new(base_url, api_key, timeout, limiter) {
        Ok(c) => Some(Arc::new(c)),
        Err(e) => {
            tracing::warn!(error = %e, "failed to build codex client; codex disabled");
            None
        }
    }
}

/// Build the shared torrent-download client when the integration is enabled.
/// Returns `None` when disabled, when `base_url` is missing (config validation
/// normally prevents this), when `kind` is unsupported, or when the reqwest
/// client fails to build (logged). v1 supports only `kind = "rutorrent"`.
fn build_download_client(
    download: &td_config::DownloadConfig,
    limiter: Arc<td_http::HttpLimiter>,
) -> Option<Arc<dyn td_download::DownloadClient>> {
    if !download.enabled {
        return None;
    }
    let Some(base_url) = download.normalized_base_url() else {
        tracing::warn!("download.enabled is true but base_url is missing; download disabled");
        return None;
    };
    if download.kind != "rutorrent" {
        tracing::warn!(
            kind = %download.kind,
            "unsupported download.kind (only \"rutorrent\" is supported); download disabled"
        );
        return None;
    }
    let timeout = std::time::Duration::from_secs(download.timeout_seconds as u64);
    match td_download::RuTorrentClient::new(
        base_url,
        download.username.clone(),
        download.password.clone(),
        timeout,
        limiter,
    ) {
        Ok(c) => Some(Arc::new(c) as Arc<dyn td_download::DownloadClient>),
        Err(e) => {
            tracing::warn!(error = %e, "failed to build rutorrent client; download disabled");
            None
        }
    }
}

/// One-shot, off-the-runtime probe of the torrent client's reachability.
/// Records the result into `download_status` (and logs it) so the admin
/// Download page reflects connectivity immediately at boot instead of showing
/// "never tested". `None` client (integration disabled) is a no-op. The probe
/// records under the `launch` trigger, so it only appends history on a real
/// reachability transition.
fn spawn_download_startup_probe(
    client: Option<Arc<dyn td_download::DownloadClient>>,
    db: DatabaseConnection,
) {
    let Some(client) = client else { return };
    tokio::spawn(async move {
        let now = chrono::Utc::now().timestamp();
        let (reachable, error) = match client.test_connection().await {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e.to_string())),
        };
        if let Err(e) = td_db::repos::download_status_repo::record_check(
            &db,
            reachable,
            error.as_deref(),
            now,
            td_db::repos::TRIGGER_LAUNCH,
        )
        .await
        {
            tracing::warn!(error = ?e, "failed to record download startup probe");
        }
        if reachable {
            tracing::info!("download client reachable at startup");
        } else {
            tracing::warn!(
                error = ?error,
                "download client unreachable at startup; the health cron / manual test will retry"
            );
        }
    });
}

/// One-shot, off-the-runtime probe of Codex's public `GET /api/v1/info`.
/// Records the result into `codex_status` (and logs it) so the admin panel
/// reflects reachability immediately at boot instead of showing the default
/// "unreachable" until the first cron tick. A success proves reachability and
/// version only — not that the api_key is valid; the first authenticated sweep
/// validates credentials.
fn spawn_codex_startup_probe(
    client: Option<Arc<td_codex::CodexClient>>,
    db: DatabaseConnection,
    base_url: String,
) {
    let Some(client) = client else { return };
    tokio::spawn(async move {
        let now = chrono::Utc::now().timestamp();
        match client.info().await {
            Ok(info) => {
                let _ = td_db::repos::codex_status_repo::record_preflight(
                    &db,
                    true,
                    Some(&info.name),
                    Some(&info.version),
                    None,
                    now,
                    td_db::repos::TRIGGER_LAUNCH,
                )
                .await;
                tracing::info!(
                    codex.name = %info.name,
                    codex.version = %info.version,
                    base_url = %base_url,
                    "codex reachable (api_key not yet validated; first sync will confirm)"
                );
            }
            Err(e) => {
                let _ = td_db::repos::codex_status_repo::record_preflight(
                    &db,
                    false,
                    None,
                    None,
                    Some(&e.to_string()),
                    now,
                    td_db::repos::TRIGGER_LAUNCH,
                )
                .await;
                tracing::warn!(
                    error = %e,
                    base_url = %base_url,
                    "codex unreachable at startup; the sync job will retry"
                );
            }
        }
    });
}

/// Inspect every registered provider and, for those that don't yet have a
/// loaded offline cache, spawn an off-the-runtime task to perform a one-shot
/// refresh. Providers that don't support an offline cache return
/// `RefreshStatus::NotSupported`, which the tick treats as a no-op.
///
/// Goes through [`td_scheduler::dispatch::try_dispatch`] so the per-provider
/// lock is held for the lifetime of the refresh — a cron tick (or an
/// admin manual trigger) that fires before the startup refresh finishes
/// gets honestly reported as skipped instead of racing.
fn spawn_startup_refreshes(
    metadata: Arc<MetadataRegistry>,
    db: DatabaseConnection,
    locks: Arc<JobLocks>,
    events: tokio::sync::broadcast::Sender<td_scheduler::JobEvent>,
) {
    for (id, provider) in metadata.iter() {
        let id = id.to_string();
        let provider = provider.clone();
        let db = db.clone();
        let locks = locks.clone();
        let events = events.clone();
        tokio::spawn(async move {
            if provider.offline_cache_loaded().await {
                tracing::debug!(provider = %id, "offline cache already loaded; skipping startup refresh");
                return;
            }
            tracing::info!(provider = %id, "no offline cache on disk; refreshing at startup");
            let lock = locks.provider_lock(&id);
            let started_at_ts = chrono::Utc::now().timestamp();
            let db_for_skip = db.clone();
            let id_for_skip = id.clone();
            let events_for_work = events.clone();
            td_scheduler::dispatch::try_dispatch(
                &events,
                lock,
                td_scheduler::JobKind::Provider,
                id.clone(),
                move || async move {
                    refresh_provider_cache::record_skipped(
                        &db_for_skip,
                        &id_for_skip,
                        started_at_ts,
                        trigger::STARTUP,
                    )
                    .await;
                },
                move || async move {
                    refresh_provider_cache::run_tick(
                        provider,
                        db,
                        events_for_work,
                        trigger::STARTUP,
                    )
                    .await;
                    td_scheduler::JobResult {
                        triggered: true,
                        skipped: false,
                        ..Default::default()
                    }
                },
            );
        });
    }
}
