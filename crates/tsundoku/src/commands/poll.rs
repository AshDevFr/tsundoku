//! `tsundoku poll [--source name]`.
//!
//! One-shot discovery poll: reads the previous `source_state` row, calls
//! the source's `poll()`, persists each [`DiscoveredRelease`] via
//! [`td_db::repos::releases_repo::persist_discovered`], and writes the new
//! `source_state` row (ETag + last-success markers, plus a short summary).
//!
//! Resolution is intentionally out of scope: this command leaves every
//! persisted release at `resolution_status='unresolved'` for the resolver
//! to pick up later.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use sea_orm::{DatabaseConnection, Set};
use td_db::entities::source_state;
use td_db::repos::{releases_repo, sources_repo};
use td_source::{DiscoverySource, PollContext, PollOutcome};

pub async fn run(config_path: PathBuf, source_name: Option<String>) -> Result<()> {
    let cfg = td_config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    super::init_tracing(&cfg);

    let db = td_db::connect(&cfg).await?;
    td_db::run_migrations(&db).await?;
    let registry = crate::source_registry::build_registry(&cfg)?;

    if registry.is_empty() {
        anyhow::bail!(
            "no discovery sources are configured; add at least one [[sources]] block to {}",
            config_path.display()
        );
    }

    let targets: Vec<&Arc<dyn DiscoverySource>> = match source_name.as_deref() {
        Some(name) => {
            let s = registry
                .get(name)
                .ok_or_else(|| anyhow::anyhow!("source {name:?} is not registered"))?;
            vec![s]
        }
        None => registry.iter().map(|(_, s)| s).collect(),
    };

    let mut summary_rows = Vec::with_capacity(targets.len());
    for source in targets {
        let summary = poll_one(&db, source.as_ref()).await;
        summary_rows.push((
            source.kind().to_string(),
            source.name().to_string(),
            summary,
        ));
    }

    render_summary(&summary_rows);
    Ok(())
}

async fn poll_one(db: &DatabaseConnection, source: &dyn DiscoverySource) -> PollSummary {
    let state = match sources_repo::get(db, source.kind(), source.name()).await {
        Ok(Some(s)) => Some(s),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = ?e, "failed to load source_state; proceeding from empty");
            None
        }
    };
    let ctx = state_to_context(state.as_ref());

    let started_at = Utc::now();
    let outcome = match source.poll(&ctx).await {
        Ok(o) => o,
        Err(e) => {
            let summary = PollSummary::failed(e.to_string());
            persist_state(db, source, &state, &ctx, &summary, started_at).await;
            return summary;
        }
    };

    let count = outcome.releases.len();
    let mut persisted = 0usize;
    let mut errors = Vec::new();
    for release in &outcome.releases {
        match releases_repo::persist_discovered(db, release, started_at.timestamp()).await {
            Ok(_) => persisted += 1,
            Err(e) => {
                tracing::error!(error = ?e, external_id = %release.external_id, "failed to persist release");
                errors.push(release.external_id.clone());
            }
        }
    }

    let summary = PollSummary::succeeded(SuccessSummary {
        fetched: count,
        persisted,
        errors: errors.len(),
        not_modified: outcome.not_modified,
    });
    persist_state_from_outcome(db, source, &outcome, &summary, started_at).await;
    summary
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

async fn persist_state_from_outcome(
    db: &DatabaseConnection,
    source: &dyn DiscoverySource,
    outcome: &PollOutcome,
    summary: &PollSummary,
    started_at: chrono::DateTime<Utc>,
) {
    let model = source_state::ActiveModel {
        source_kind: Set(source.kind().to_string()),
        source_name: Set(source.name().to_string()),
        etag: Set(outcome.new_etag.clone()),
        cursor: Set(outcome.new_cursor.clone()),
        last_polled_at: Set(Some(started_at.timestamp())),
        last_success_at: Set(Some(started_at.timestamp())),
        last_error: Set(None),
        last_summary: Set(Some(summary.short())),
    };
    if let Err(e) = sources_repo::upsert(db, model).await {
        tracing::warn!(error = ?e, "failed to upsert source_state after successful poll");
    }
}

async fn persist_state(
    db: &DatabaseConnection,
    source: &dyn DiscoverySource,
    previous: &Option<source_state::Model>,
    ctx: &PollContext,
    summary: &PollSummary,
    started_at: chrono::DateTime<Utc>,
) {
    let model = source_state::ActiveModel {
        source_kind: Set(source.kind().to_string()),
        source_name: Set(source.name().to_string()),
        etag: Set(ctx.etag.clone()),
        cursor: Set(ctx.cursor.clone()),
        last_polled_at: Set(Some(started_at.timestamp())),
        last_success_at: Set(previous.as_ref().and_then(|p| p.last_success_at)),
        last_error: Set(summary.error.clone()),
        last_summary: Set(Some(summary.short())),
    };
    if let Err(e) = sources_repo::upsert(db, model).await {
        tracing::warn!(error = ?e, "failed to upsert source_state after failed poll");
    }
}

#[derive(Debug)]
struct PollSummary {
    success: Option<SuccessSummary>,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct SuccessSummary {
    fetched: usize,
    persisted: usize,
    errors: usize,
    not_modified: bool,
}

impl PollSummary {
    fn succeeded(s: SuccessSummary) -> Self {
        Self {
            success: Some(s),
            error: None,
        }
    }
    fn failed(message: String) -> Self {
        Self {
            success: None,
            error: Some(message),
        }
    }
    fn short(&self) -> String {
        if let Some(err) = &self.error {
            return format!("error: {err}");
        }
        let Some(s) = self.success else {
            return "unknown".into();
        };
        if s.not_modified {
            return "ok: not modified".into();
        }
        if s.errors == 0 {
            format!("ok: {} fetched, {} persisted", s.fetched, s.persisted)
        } else {
            format!(
                "ok: {} fetched, {} persisted, {} errors",
                s.fetched, s.persisted, s.errors
            )
        }
    }
}

fn render_summary(rows: &[(String, String, PollSummary)]) {
    println!("\npoll summary:");
    for (kind, name, summary) in rows {
        println!("  {kind}/{name}  →  {}", summary.short());
    }
}
