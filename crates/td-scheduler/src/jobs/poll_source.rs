//! Scheduled poll job for a single [`DiscoverySource`].
//!
//! Each tick:
//! 1. Read the previous `source_state` row to recover the ETag / cursor.
//! 2. Call `source.poll()`.
//! 3. Persist every release via
//!    [`td_db::repos::releases_repo::persist_discovered`].
//! 4. Run the resolution pipeline on each persisted release.
//! 5. Upsert `source_state` with the new ETag/cursor and a short summary.
//!
//! Errors at any step are logged and recorded on `source_state.last_error`
//! but never propagate out of the tick — a failing source must not poison
//! the scheduler for the others.
//!
//! Per-source contention (overlapping cron fires + manual triggers) is
//! handled one layer up by [`crate::dispatch::try_dispatch`], which holds
//! the per-key mutex for the lifetime of the spawned task. `run_tick`
//! assumes that lock is already held by its caller and never tries to
//! re-acquire it.

use std::sync::Arc;

use std::time::Instant;

use anyhow::{Result, anyhow};
use chrono::{TimeZone, Utc};
use sea_orm::{DatabaseConnection, Set, TransactionTrait};
use td_config::IngestionConfig;
use td_db::entities::source_state;
use td_db::repos::run_metrics_repo::{self, PollRunCounts, ProgressTable};
use td_db::repos::{releases_repo, sources_repo};
use td_metadata::MetadataRegistry;
use td_resolution::Resolver;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;
use td_source::{DiscoveredRelease, DiscoverySource, PollContext, PollOutcome};
use tokio::sync::broadcast;
use tokio_cron_scheduler::{Job, JobSchedulerError};

use crate::JobLocks;
use crate::dispatch;
use crate::error_kind;
use crate::events::{JobEvent, JobKind, JobResult};
use crate::jobs::outcomes::OutcomeBreakdown;
use crate::jobs::progress::ProgressHandle;

/// Build a scheduled poll job for `source`. The cron must already be in
/// the 6- or 7-field form expected by tokio-cron-scheduler (the bootstrap
/// in [`crate::Scheduler::build`] normalises 5-field strings before getting
/// here).
#[allow(clippy::too_many_arguments)]
pub fn build(
    cron: &str,
    source: Arc<dyn DiscoverySource>,
    db: DatabaseConnection,
    metadata: Arc<MetadataRegistry>,
    ingestion: IngestionConfig,
    locks: Arc<JobLocks>,
    query_builder: Arc<QueryBuilder>,
    mangaupdates_redirector: Option<Arc<MangaUpdatesRedirector>>,
    events: broadcast::Sender<JobEvent>,
) -> Result<Job> {
    let job = Job::new_async(cron, move |_uuid, _scheduler| {
        let source = source.clone();
        let db = db.clone();
        let metadata = metadata.clone();
        let ingestion = ingestion.clone();
        let locks = locks.clone();
        let query_builder = query_builder.clone();
        let mu_redirector = mangaupdates_redirector.clone();
        let events = events.clone();
        Box::pin(async move {
            let name = source.name().to_string();
            let kind = source.kind().to_string();
            let lock = locks.source_lock(&name);
            let started_at_ts = Utc::now().timestamp();
            let trigger = run_metrics_repo::trigger::CRON;
            let db_for_skip = db.clone();
            let name_for_skip = name.clone();
            let kind_for_skip = kind.clone();
            let events_for_work = events.clone();
            dispatch::try_dispatch(
                &events,
                lock,
                JobKind::Source,
                name.clone(),
                move || async move {
                    record_skipped(
                        &db_for_skip,
                        &name_for_skip,
                        &kind_for_skip,
                        started_at_ts,
                        trigger,
                    )
                    .await;
                },
                move || async move {
                    run_tick(
                        source,
                        db,
                        metadata,
                        ingestion,
                        query_builder,
                        mu_redirector,
                        events_for_work,
                        trigger,
                    )
                    .await;
                    JobResult {
                        triggered: true,
                        skipped: false,
                        ..Default::default()
                    }
                },
            );
        })
    })
    .map_err(|e: JobSchedulerError| anyhow!("building poll-source job: {e}"))?;
    Ok(job)
}

/// One tick of the poll-and-resolve loop. Public for the integration tests
/// in [`crate::jobs::poll_source::tests`] and for the CLI / API trigger
/// paths; the scheduled job uses the same code path so behaviour matches.
///
/// Callers are responsible for per-source contention via
/// [`crate::dispatch::try_dispatch`]; this body does **not** acquire the
/// per-source lock itself. Calling it directly (CLI subcommands, tests)
/// is fine because there are no concurrent triggers in those contexts.
#[allow(clippy::too_many_arguments)]
pub async fn run_tick(
    source: Arc<dyn DiscoverySource>,
    db: DatabaseConnection,
    metadata: Arc<MetadataRegistry>,
    ingestion: IngestionConfig,
    query_builder: Arc<QueryBuilder>,
    mangaupdates_redirector: Option<Arc<MangaUpdatesRedirector>>,
    events: broadcast::Sender<JobEvent>,
    trigger: &str,
) {
    let kind = source.kind().to_string();
    let name = source.name().to_string();
    let started_at = Utc::now();
    let started_at_ts = started_at.timestamp();
    tracing::info!(source = %name, kind = %kind, trigger = %trigger, "poll tick started");

    // Insert a "running" row up front so an aborted process (oom, SIGKILL)
    // leaves a stale row the admin UI can call out, rather than no record.
    let metrics_id =
        match run_metrics_repo::start_poll_run(&db, &name, &kind, started_at_ts, trigger).await {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::warn!(error = ?e, source = %name, "failed to record poll_run start");
                None
            }
        };

    // Live progress: SSE every tick, DB writes throttled. `metrics_id =
    // None` (insert failed) builds a no-op handle so the loop still runs.
    let progress = ProgressHandle::new(
        db.clone(),
        ProgressTable::PollRuns,
        metrics_id,
        events,
        JobKind::Source,
        &name,
    );

    let prev_state = match sources_repo::get(&db, &kind, &name).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = ?e, source = %name, "failed to read source_state; proceeding from empty");
            None
        }
    };
    let mut ctx = state_to_context(prev_state.as_ref());
    ctx.recently_seen = load_recently_seen(&db, &kind, &name).await;

    // Time only the outbound HTTP call, not the surrounding persist/resolve
    // loop — the admin metrics card uses this as the "feed health" signal.
    let fetch_started = Instant::now();
    let poll_result = source.poll(&ctx).await;
    let fetch_duration_ms = fetch_started.elapsed().as_millis() as i64;
    let mut outcome = match poll_result {
        Ok(o) => o,
        Err(e) => {
            let err: anyhow::Error = anyhow::Error::new(e).context(format!("poll {name} failed"));
            tracing::warn!(error = ?err, source = %name, "poll failed");
            let summary = format!("error: {err}");
            persist_failure(
                &db,
                &kind,
                &name,
                prev_state.as_ref(),
                &ctx,
                &summary,
                started_at,
            )
            .await;
            let kind_class = error_kind::classify_anyhow(&err);
            finalize_metrics(
                &db,
                metrics_id,
                run_metrics_repo::status::FAILURE,
                PollRunCounts {
                    fetch_duration_ms: Some(fetch_duration_ms),
                    ..Default::default()
                },
                Some(&summary),
                Some(kind_class),
            )
            .await;
            return;
        }
    };

    let fetched = outcome.releases.len();
    tracing::info!(
        source = %name,
        fetched,
        fetch_duration_ms,
        not_modified = outcome.not_modified,
        "feed parsed; starting enrich + persist + resolve"
    );
    progress.set_total(fetched as u64).await;

    // Enrich + persist + resolve in chunks of `ingestion.poll_write_batch_size`.
    // Per chunk: (A) enrich every item with the source's HTTP detail call —
    // strictly outside the DB transaction so we don't hold a SQLite write
    // lock during HTTP, (B) open one transaction and upsert every item
    // (per-item upsert errors log + count + continue, the rest of the
    // chunk still commits), (C) commit and run the resolver per persisted
    // id. A commit failure logs and counts the whole chunk as
    // persist_errors; the next tick re-fetches and `persist_discovered`
    // is idempotent on `(source_kind, external_id)` so nothing is lost.
    // Enrich errors are non-fatal — we persist with RSS-only data per
    // trait contract.
    let mut resolver =
        Resolver::new(db.clone(), metadata, ingestion.clone()).with_query_builder(query_builder);
    if let Some(r) = mangaupdates_redirector {
        resolver = resolver.with_mangaupdates_redirector(r);
    }

    const PROGRESS_EVERY: usize = 25;
    let batch_size = ingestion.poll_write_batch_size.max(1) as usize;
    let mut persisted = 0usize;
    let mut persist_errors = 0usize;
    let mut enrich_errors = 0usize;
    let mut resolve_errors = 0usize;
    let mut breakdown = OutcomeBreakdown::default();
    let mut processed = 0usize;
    let mut enrich_total_ms: u128 = 0;
    let mut resolve_total_ms: u128 = 0;
    for chunk in outcome.releases.chunks_mut(batch_size) {
        // (A) Enrich the whole chunk first. No DB lock held.
        for release in chunk.iter_mut() {
            let started = Instant::now();
            let res = source.enrich(release).await;
            enrich_total_ms += started.elapsed().as_millis();
            if let Err(e) = res {
                tracing::warn!(
                    error = ?e,
                    source = %name,
                    external_id = %release.external_id,
                    "enrich failed; persisting with rss-only data"
                );
                enrich_errors += 1;
            }
        }

        // (B) Open one transaction, upsert every item, commit.
        let chunk_size = chunk.len();
        match persist_chunk(&db, chunk, started_at.timestamp()).await {
            Ok(outcome) => {
                persisted += outcome.ids.len();
                persist_errors += outcome.per_item_failures;
                let persist_failed_in_chunk = outcome.per_item_failures;

                // (C) Resolve each persisted id. Resolver writes are their
                // own per-item transactions; that's intentional, the
                // resolver touches several tables (review_candidates,
                // series_external_ids, ...) per release and the size of
                // its write set varies enormously per release.
                for id in &outcome.ids {
                    let started = Instant::now();
                    let res = resolver.resolve_one(id).await;
                    resolve_total_ms += started.elapsed().as_millis();
                    match res {
                        Ok(o) => breakdown.record(&o),
                        Err(e) => {
                            tracing::warn!(error = ?e, release_id = %id, "resolver failed; leaving release unresolved");
                            resolve_errors += 1;
                            breakdown.failed += 1;
                        }
                    }
                    processed += 1;
                    progress.tick_to(processed as u64).await;
                    if processed.is_multiple_of(PROGRESS_EVERY) && processed < fetched {
                        tracing::info!(
                            source = %name,
                            processed,
                            total = fetched,
                            "poll tick progress"
                        );
                    }
                }
                // Items that failed persist still consumed a slot; bump
                // `processed` so the pill reflects the whole chunk done.
                processed += persist_failed_in_chunk;
                progress.tick_to(processed as u64).await;
            }
            Err(e) => {
                tracing::error!(
                    error = ?e,
                    source = %name,
                    chunk_size,
                    "batch persist transaction failed; all items in batch rolled back"
                );
                persist_errors += chunk_size;
                processed += chunk_size;
                progress.tick_to(processed as u64).await;
            }
        }
    }
    progress.flush().await;
    let enrich_duration_ms = enrich_total_ms.min(i64::MAX as u128) as i64;
    let resolve_duration_ms = resolve_total_ms.min(i64::MAX as u128) as i64;

    let summary = build_summary(
        fetched,
        persisted,
        persist_errors,
        enrich_errors,
        resolve_errors,
        outcome.not_modified,
    );
    tracing::info!(source = %name, %summary, "poll tick complete");
    persist_success(&db, &kind, &name, &outcome, &summary, started_at).await;

    let counts = PollRunCounts {
        fetched: Some(fetched as i32),
        new: Some(persisted as i32),
        resolved: Some(persisted.saturating_sub(resolve_errors) as i32),
        fetch_duration_ms: Some(fetch_duration_ms),
        enrich_duration_ms: Some(enrich_duration_ms),
        resolve_duration_ms: Some(resolve_duration_ms),
        outcome_known_id: Some(breakdown.known_id),
        outcome_foreign_id: Some(breakdown.foreign_id),
        outcome_fuzzy: Some(breakdown.fuzzy),
        outcome_review: Some(breakdown.review),
        outcome_failed: Some(breakdown.failed),
    };
    finalize_metrics(
        &db,
        metrics_id,
        run_metrics_repo::status::SUCCESS,
        counts,
        None,
        None,
    )
    .await;
}

/// Outcome of `persist_chunk` when the commit succeeds. `ids` is the
/// subset of the chunk that actually landed (per-item upsert errors are
/// caught and counted as `per_item_failures` without aborting the
/// whole transaction).
struct PersistChunkOutcome {
    ids: Vec<String>,
    per_item_failures: usize,
}

/// Upsert every release in `chunk` inside one `BEGIN ... COMMIT`. Per-item
/// errors are logged and counted but do not roll the transaction back —
/// the survivors still commit. A failure at `begin()` or `commit()` is
/// surfaced via `Err`; the caller treats the whole chunk as lost in that
/// case (idempotent retry on the next tick).
async fn persist_chunk(
    db: &DatabaseConnection,
    chunk: &[DiscoveredRelease],
    observed_at: i64,
) -> Result<PersistChunkOutcome> {
    let tx = db.begin().await?;
    let mut ids = Vec::with_capacity(chunk.len());
    let mut per_item_failures = 0usize;
    for release in chunk {
        match releases_repo::persist_discovered(&tx, release, observed_at).await {
            Ok(id) => ids.push(id),
            Err(e) => {
                per_item_failures += 1;
                tracing::error!(
                    error = ?e,
                    external_id = %release.external_id,
                    "upsert failed inside batch transaction; continuing with survivors"
                );
            }
        }
    }
    tx.commit().await?;
    Ok(PersistChunkOutcome {
        ids,
        per_item_failures,
    })
}

/// Write a `poll_runs` row marked `skipped` so the admin metrics view
/// surfaces contention. Called by [`crate::dispatch::try_dispatch`] when
/// a tick lost the per-source lock.
pub async fn record_skipped(
    db: &DatabaseConnection,
    name: &str,
    kind: &str,
    started_at: i64,
    trigger: &str,
) {
    match run_metrics_repo::start_poll_run(db, name, kind, started_at, trigger).await {
        Ok(id) => {
            if let Err(e) = run_metrics_repo::finalize_poll_run(
                db,
                id,
                started_at,
                run_metrics_repo::status::SKIPPED,
                PollRunCounts::default(),
                Some("previous tick still running"),
                None,
            )
            .await
            {
                tracing::warn!(error = ?e, source = %name, "failed to record skipped poll_run");
            }
        }
        Err(e) => {
            tracing::warn!(error = ?e, source = %name, "failed to insert skipped poll_run row");
        }
    }
}

async fn finalize_metrics(
    db: &DatabaseConnection,
    id: Option<i64>,
    status: &str,
    counts: PollRunCounts,
    error_message: Option<&str>,
    error_kind: Option<&str>,
) {
    let Some(id) = id else { return };
    let finished_at = Utc::now().timestamp();
    if let Err(e) = run_metrics_repo::finalize_poll_run(
        db,
        id,
        finished_at,
        status,
        counts,
        error_message,
        error_kind,
    )
    .await
    {
        tracing::warn!(error = ?e, run_id = id, "failed to finalize poll_run row");
    }
}

/// Upper bound on the number of recent `external_id`s loaded into
/// `PollContext.recently_seen`. Generous enough to cover several pages of a
/// paginated source (Nyaa is 75 items/page) without ever materializing more
/// than a few hundred short strings.
const RECENT_SEEN_LIMIT: u64 = 500;

async fn load_recently_seen(
    db: &DatabaseConnection,
    kind: &str,
    name: &str,
) -> std::collections::HashSet<String> {
    match releases_repo::recent_external_ids(db, kind, name, RECENT_SEEN_LIMIT).await {
        Ok(ids) => ids.into_iter().collect(),
        Err(e) => {
            tracing::warn!(
                error = ?e,
                source = %name,
                "failed to load recent external_ids; proceeding without dedup hint"
            );
            std::collections::HashSet::new()
        }
    }
}

fn state_to_context(state: Option<&source_state::Model>) -> PollContext {
    let Some(state) = state else {
        return PollContext::default();
    };
    let last_success_at = state
        .last_success_at
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single());
    PollContext {
        etag: state.etag.clone(),
        cursor: state.cursor.clone(),
        last_success_at,
        ..Default::default()
    }
}

async fn persist_success(
    db: &DatabaseConnection,
    kind: &str,
    name: &str,
    outcome: &PollOutcome,
    summary: &str,
    started_at: chrono::DateTime<Utc>,
) {
    let model = source_state::ActiveModel {
        source_kind: Set(kind.to_string()),
        source_name: Set(name.to_string()),
        etag: Set(outcome.new_etag.clone()),
        cursor: Set(outcome.new_cursor.clone()),
        last_polled_at: Set(Some(started_at.timestamp())),
        last_success_at: Set(Some(started_at.timestamp())),
        last_error: Set(None),
        last_summary: Set(Some(summary.to_string())),
    };
    if let Err(e) = sources_repo::upsert(db, model).await {
        tracing::warn!(error = ?e, source = %name, "failed to upsert source_state after successful poll");
    }
}

async fn persist_failure(
    db: &DatabaseConnection,
    kind: &str,
    name: &str,
    previous: Option<&source_state::Model>,
    ctx: &PollContext,
    summary: &str,
    started_at: chrono::DateTime<Utc>,
) {
    let model = source_state::ActiveModel {
        source_kind: Set(kind.to_string()),
        source_name: Set(name.to_string()),
        etag: Set(ctx.etag.clone()),
        cursor: Set(ctx.cursor.clone()),
        last_polled_at: Set(Some(started_at.timestamp())),
        last_success_at: Set(previous.and_then(|p| p.last_success_at)),
        last_error: Set(Some(summary.to_string())),
        last_summary: Set(Some(summary.to_string())),
    };
    if let Err(e) = sources_repo::upsert(db, model).await {
        tracing::warn!(error = ?e, source = %name, "failed to upsert source_state after failed poll");
    }
}

fn build_summary(
    fetched: usize,
    persisted: usize,
    persist_errors: usize,
    enrich_errors: usize,
    resolve_errors: usize,
    not_modified: bool,
) -> String {
    if not_modified {
        return "ok: not modified".into();
    }
    let mut s = format!("ok: {fetched} fetched, {persisted} persisted");
    if persist_errors > 0 {
        s.push_str(&format!(", {persist_errors} persist_errors"));
    }
    if enrich_errors > 0 {
        s.push_str(&format!(", {enrich_errors} enrich_errors"));
    }
    if resolve_errors > 0 {
        s.push_str(&format!(", {resolve_errors} resolve_errors"));
    }
    s
}
