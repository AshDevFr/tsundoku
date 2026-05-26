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

use anyhow::{Result, anyhow};
use chrono::{TimeZone, Utc};
use sea_orm::{DatabaseConnection, Set};
use td_config::IngestionConfig;
use td_db::entities::source_state;
use td_db::repos::{releases_repo, sources_repo};
use td_metadata::MetadataRegistry;
use td_resolution::Resolver;
use td_source::{DiscoverySource, PollContext, PollOutcome};
use tokio_cron_scheduler::{Job, JobSchedulerError};

use crate::JobLocks;

/// Build a scheduled poll job for `source`. The cron must already be in
/// the 6- or 7-field form expected by tokio-cron-scheduler (the bootstrap
/// in [`crate::Scheduler::build`] normalises 5-field strings before getting
/// here).
pub fn build(
    cron: &str,
    source: Arc<dyn DiscoverySource>,
    db: DatabaseConnection,
    metadata: Arc<MetadataRegistry>,
    ingestion: IngestionConfig,
    locks: Arc<JobLocks>,
) -> Result<Job> {
    let job = Job::new_async(cron, move |_uuid, _scheduler| {
        let source = source.clone();
        let db = db.clone();
        let metadata = metadata.clone();
        let ingestion = ingestion.clone();
        let locks = locks.clone();
        Box::pin(async move {
            run_tick(source, db, metadata, ingestion, locks).await;
        })
    })
    .map_err(|e: JobSchedulerError| anyhow!("building poll-source job: {e}"))?;
    Ok(job)
}

/// One tick of the poll-and-resolve loop. Public for the integration tests
/// in [`crate::jobs::poll_source::tests`] and for any future "trigger
/// manually" CLI; the scheduled job uses the same code path so behaviour
/// matches.
pub async fn run_tick(
    source: Arc<dyn DiscoverySource>,
    db: DatabaseConnection,
    metadata: Arc<MetadataRegistry>,
    ingestion: IngestionConfig,
    locks: Arc<JobLocks>,
) {
    let kind = source.kind().to_string();
    let name = source.name().to_string();

    let lock = locks.source_lock(&name);
    let Ok(_guard) = lock.try_lock() else {
        tracing::debug!(source = %name, "previous tick still running; skipping");
        return;
    };

    let started_at = Utc::now();
    let prev_state = match sources_repo::get(&db, &kind, &name).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = ?e, source = %name, "failed to read source_state; proceeding from empty");
            None
        }
    };
    let ctx = state_to_context(prev_state.as_ref());

    let outcome = match source.poll(&ctx).await {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = ?e, source = %name, "poll failed");
            let summary = format!("error: {e}");
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
            return;
        }
    };

    let fetched = outcome.releases.len();
    let mut persisted_ids = Vec::with_capacity(fetched);
    let mut persist_errors = 0usize;
    for release in &outcome.releases {
        match releases_repo::persist_discovered(&db, release, started_at.timestamp()).await {
            Ok(id) => persisted_ids.push(id),
            Err(e) => {
                tracing::error!(
                    error = ?e,
                    source = %name,
                    external_id = %release.external_id,
                    "failed to persist release"
                );
                persist_errors += 1;
            }
        }
    }

    let resolver = Resolver::new(db.clone(), metadata, ingestion);
    let mut resolve_errors = 0usize;
    for id in &persisted_ids {
        if let Err(e) = resolver.resolve_one(id).await {
            tracing::warn!(error = ?e, release_id = %id, "resolver failed; leaving release unresolved");
            resolve_errors += 1;
        }
    }

    let summary = build_summary(
        fetched,
        persisted_ids.len(),
        persist_errors,
        resolve_errors,
        outcome.not_modified,
    );
    tracing::info!(source = %name, %summary, "poll tick complete");
    persist_success(&db, &kind, &name, &outcome, &summary, started_at).await;
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
    if resolve_errors > 0 {
        s.push_str(&format!(", {resolve_errors} resolve_errors"));
    }
    s
}
