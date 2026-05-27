//! Scheduled poll job for a single [`DiscoverySource`].
//!
//! Each tick:
//! 1. Acquire the per-source mutex (skip the tick if another run is in
//!    flight — overlapping cron fires are dropped, not queued).
//! 2. Read the previous `source_state` row to recover the ETag / cursor.
//! 3. Call `source.poll()`.
//! 4. Persist every release via
//!    [`td_db::repos::releases_repo::persist_discovered`].
//! 5. Run the resolution pipeline on each persisted release.
//! 6. Upsert `source_state` with the new ETag/cursor and a short summary.
//!
//! Errors at any step are logged and recorded on `source_state.last_error`
//! but never propagate out of the tick — a failing source must not poison
//! the scheduler for the others.

use std::sync::Arc;

use std::time::Instant;

use anyhow::{Result, anyhow};
use chrono::{TimeZone, Utc};
use sea_orm::{DatabaseConnection, Set};
use td_config::IngestionConfig;
use td_db::entities::source_state;
use td_db::repos::run_metrics_repo::{self, PollRunCounts};
use td_db::repos::{releases_repo, sources_repo};
use td_metadata::MetadataRegistry;
use td_resolution::Resolver;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::pipeline::{ResolutionOutcome, ResolutionPath, ResolutionStatus};
use td_resolution::query_builder::QueryBuilder;
use td_source::{DiscoverySource, PollContext, PollOutcome};
use tokio_cron_scheduler::{Job, JobSchedulerError};

use crate::JobLocks;
use crate::error_kind;

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
) -> Result<Job> {
    let job = Job::new_async(cron, move |_uuid, _scheduler| {
        let source = source.clone();
        let db = db.clone();
        let metadata = metadata.clone();
        let ingestion = ingestion.clone();
        let locks = locks.clone();
        let query_builder = query_builder.clone();
        let mu_redirector = mangaupdates_redirector.clone();
        Box::pin(async move {
            run_tick(
                source,
                db,
                metadata,
                ingestion,
                locks,
                query_builder,
                mu_redirector,
                run_metrics_repo::trigger::CRON,
            )
            .await;
        })
    })
    .map_err(|e: JobSchedulerError| anyhow!("building poll-source job: {e}"))?;
    Ok(job)
}

/// One tick of the poll-and-resolve loop. Public for the integration tests
/// in [`crate::jobs::poll_source::tests`] and for any future "trigger
/// manually" CLI; the scheduled job uses the same code path so behaviour
/// matches.
#[allow(clippy::too_many_arguments)]
pub async fn run_tick(
    source: Arc<dyn DiscoverySource>,
    db: DatabaseConnection,
    metadata: Arc<MetadataRegistry>,
    ingestion: IngestionConfig,
    locks: Arc<JobLocks>,
    query_builder: Arc<QueryBuilder>,
    mangaupdates_redirector: Option<Arc<MangaUpdatesRedirector>>,
    trigger: &str,
) {
    let kind = source.kind().to_string();
    let name = source.name().to_string();
    let started_at = Utc::now();
    let started_at_ts = started_at.timestamp();
    tracing::info!(source = %name, kind = %kind, trigger = %trigger, "poll tick started");

    let lock = locks.source_lock(&name);
    let Ok(_guard) = lock.try_lock() else {
        tracing::debug!(source = %name, "previous tick still running; skipping");
        // Record the skip so the admin metrics view can still surface
        // contention (e.g. cron ticks piling up behind a slow source).
        record_skipped(&db, &name, &kind, started_at_ts, trigger).await;
        return;
    };

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

    // Enrich + persist + resolve per release. Each iteration commits to
    // SQLite before moving on, so a mid-loop crash leaves the work done
    // so far visible in the UI; the next tick re-fetches (the ETag is
    // only advanced after the whole walk) and re-persisting is
    // idempotent on `(source_kind, external_id)`. Enrich errors are
    // logged but non-fatal by trait contract — we persist with the
    // RSS-only data. A resolver Err is logged and counted as a `failed`
    // outcome so the breakdown ties out against the persisted release
    // count.
    let mut resolver =
        Resolver::new(db.clone(), metadata, ingestion).with_query_builder(query_builder);
    if let Some(r) = mangaupdates_redirector {
        resolver = resolver.with_mangaupdates_redirector(r);
    }

    const PROGRESS_EVERY: usize = 25;
    let mut persisted = 0usize;
    let mut persist_errors = 0usize;
    let mut enrich_errors = 0usize;
    let mut resolve_errors = 0usize;
    let mut breakdown = OutcomeBreakdown::default();
    for (idx, release) in outcome.releases.iter_mut().enumerate() {
        if let Err(e) = source.enrich(release).await {
            tracing::warn!(
                error = ?e,
                source = %name,
                external_id = %release.external_id,
                "enrich failed; persisting with rss-only data"
            );
            enrich_errors += 1;
        }
        let id = match releases_repo::persist_discovered(&db, release, started_at.timestamp()).await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(
                    error = ?e,
                    source = %name,
                    external_id = %release.external_id,
                    "failed to persist release"
                );
                persist_errors += 1;
                continue;
            }
        };
        persisted += 1;
        match resolver.resolve_one(&id).await {
            Ok(o) => breakdown.record(&o),
            Err(e) => {
                tracing::warn!(error = ?e, release_id = %id, "resolver failed; leaving release unresolved");
                resolve_errors += 1;
                breakdown.failed += 1;
            }
        }
        let processed = idx + 1;
        if processed % PROGRESS_EVERY == 0 && processed < fetched {
            tracing::info!(
                source = %name,
                processed,
                total = fetched,
                "poll tick progress"
            );
        }
    }

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

#[derive(Default)]
struct OutcomeBreakdown {
    known_id: i32,
    foreign_id: i32,
    fuzzy: i32,
    review: i32,
    failed: i32,
}

impl OutcomeBreakdown {
    fn record(&mut self, outcome: &ResolutionOutcome) {
        match (outcome.path, outcome.status) {
            (Some(ResolutionPath::KnownExternalId), ResolutionStatus::Resolved) => {
                self.known_id += 1
            }
            (Some(ResolutionPath::ForeignIdLookup), ResolutionStatus::Resolved) => {
                self.foreign_id += 1
            }
            (Some(ResolutionPath::FuzzyTitle), ResolutionStatus::Resolved) => self.fuzzy += 1,
            (_, ResolutionStatus::ReviewPending) | (_, ResolutionStatus::Ambiguous) => {
                self.review += 1
            }
            _ => self.failed += 1,
        }
    }
}

async fn record_skipped(
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
