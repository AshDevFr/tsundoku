//! Codex presence sync job.
//!
//! One sweep does: a public `/info` preflight (skip the rest on failure,
//! leaving existing links untouched), then the authenticated `external-index`
//! sweep, matching each Codex external id to a local series via
//! `series_external_ids` and upserting an `auto` link. Stale `auto` links
//! (matches that disappeared) are pruned; `manual` links have their counts
//! refreshed by `codex_series_uuid`. Connection health lands in `codex_status`
//! so the admin UI can show it.
//!
//! Contention (overlapping cron + manual `POST /codex/refresh`) is handled one
//! layer up by [`crate::dispatch::try_dispatch`] on the single codex lock; this
//! body does not acquire it.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use sea_orm::DatabaseConnection;
use td_codex::{
    CodexClient, CodexError, DEFAULT_SWEEP_PAGE_SIZE, ExternalIndexItem, normalize_source,
};
use td_db::repos::codex_link_repo::AutoLink;
use td_db::repos::{codex_link_repo, codex_status_repo, series_external_ids_repo};
use tokio::sync::broadcast;
use tokio_cron_scheduler::{Job, JobSchedulerError};

use crate::JobLocks;
use crate::dispatch;
use crate::events::{JobEvent, JobKind, JobResult};

/// The fixed dispatch/event key for the (single) codex sync job.
pub const JOB_KEY: &str = "codex";

/// Codex's per-series counts: `(local_max_volume, local_max_chapter,
/// volumes_owned)`. Keyed by `codex_series_uuid` to refresh manual links.
type Counts = (Option<f64>, Option<f64>, Option<i64>);

pub fn build(
    cron: &str,
    client: Arc<CodexClient>,
    db: DatabaseConnection,
    locks: Arc<JobLocks>,
    events: broadcast::Sender<JobEvent>,
) -> Result<Job> {
    let job = Job::new_async(cron, move |_uuid, _scheduler| {
        let client = client.clone();
        let db = db.clone();
        let locks = locks.clone();
        let events = events.clone();
        Box::pin(async move {
            let lock = locks.codex_sync_lock();
            dispatch::try_dispatch(
                &events,
                lock,
                JobKind::Codex,
                JOB_KEY,
                || async {},
                move || async move {
                    run_tick(client, db).await;
                    JobResult {
                        triggered: true,
                        skipped: false,
                        ..Default::default()
                    }
                },
            );
        })
    })
    .map_err(|e: JobSchedulerError| anyhow!("building codex sync job: {e}"))?;
    Ok(job)
}

/// One sync tick. Public so the manual API trigger can drive it directly.
/// Contention is handled by the caller via [`crate::dispatch::try_dispatch`];
/// `run_tick` does not acquire the codex lock itself.
pub async fn run_tick(client: Arc<CodexClient>, db: DatabaseConnection) {
    let now = chrono::Utc::now().timestamp();

    // Preflight: a failed /info means Codex is down or misconfigured. Record
    // it and bail before the heavier authenticated sweep — existing links and
    // last_success_at stay intact so stale-but-valid data survives an outage.
    match client.info().await {
        Ok(info) => {
            if let Err(e) = codex_status_repo::set_preflight(
                &db,
                true,
                Some(&info.name),
                Some(&info.version),
                now,
            )
            .await
            {
                tracing::warn!(error = ?e, "failed to record codex preflight status");
            }
            tracing::info!(
                codex.name = %info.name,
                codex.version = %info.version,
                "codex preflight ok (api_key validated by the sweep, not /info)"
            );
        }
        Err(e) => {
            let _ = codex_status_repo::set_preflight_unreachable(&db, &e.to_string(), now).await;
            tracing::warn!(error = %e, "codex preflight failed; skipping sweep, preserving links");
            return;
        }
    }

    // Authenticated sweep. 401/403 are classified into a durable auth_state;
    // any failure preserves existing links rather than wiping them.
    let items = match client.fetch_all(DEFAULT_SWEEP_PAGE_SIZE).await {
        Ok(items) => items,
        Err(e) => {
            match classify_sweep_error(&e) {
                (Some(auth_state), msg) => {
                    let _ = codex_status_repo::set_auth_state(&db, auth_state, Some(&msg)).await;
                }
                (None, msg) => {
                    let _ = codex_status_repo::set_error(&db, &msg).await;
                }
            }
            tracing::warn!(error = %e, "codex external-index sweep failed; preserving existing links");
            return;
        }
    };

    let swept = items.len();
    match apply_sweep(&db, &items, now).await {
        Ok(linked) => {
            if let Err(e) =
                codex_status_repo::set_success(&db, swept as i64, linked as i64, now).await
            {
                tracing::warn!(error = ?e, "failed to record codex sync success");
            }
            tracing::info!(swept, linked, "codex sync complete");
        }
        Err(e) => {
            let _ = codex_status_repo::set_error(&db, &format!("applying sweep: {e}")).await;
            tracing::warn!(error = ?e, "codex sync failed while applying sweep to the database");
        }
    }
}

/// Apply a fetched set of Codex items to the local link table: match each
/// item's external ids to a local series and upsert an `auto` link, prune
/// stale `auto` links, then refresh `manual` link counts by uuid. Returns the
/// total link count afterwards. Pure DB work — no network — so it is unit
/// tested directly with hand-built items.
pub async fn apply_sweep(
    db: &DatabaseConnection,
    items: &[ExternalIndexItem],
    now: i64,
) -> Result<usize> {
    let mut alive: Vec<i32> = Vec::new();
    // codex_series_uuid -> counts, for refreshing manual links below.
    let mut counts_by_uuid: HashMap<&str, Counts> = HashMap::new();

    for item in items {
        counts_by_uuid.insert(
            item.id.as_str(),
            (
                item.local_max_volume,
                item.local_max_chapter,
                item.volumes_owned,
            ),
        );

        // First external id that resolves to a local series wins the auto match.
        for ext in &item.external_ids {
            let Some(provider) = normalize_source(&ext.source) else {
                continue;
            };
            if let Some(series_id) =
                series_external_ids_repo::find_series_id(db, &provider, &ext.external_id).await?
            {
                codex_link_repo::upsert_auto(
                    db,
                    &AutoLink {
                        series_id,
                        codex_series_uuid: item.id.clone(),
                        local_max_volume: item.local_max_volume,
                        local_max_chapter: item.local_max_chapter,
                        volumes_owned: item.volumes_owned,
                        matched_provider: provider,
                        matched_external_id: ext.external_id.clone(),
                        synced_at: now,
                    },
                )
                .await?;
                alive.push(series_id);
                break;
            }
        }
    }

    // Drop auto links whose match disappeared this sweep; manual links survive.
    codex_link_repo::delete_stale_auto(db, &alive).await?;

    // Refresh manual links' counts from the swept data (matched by uuid). A
    // manual link is created without counts, so this is how it gets a
    // comparable status instead of forever showing "present".
    for link in codex_link_repo::list_manual(db).await? {
        if let Some((vol, chap, owned)) = counts_by_uuid.get(link.codex_series_uuid.as_str()) {
            codex_link_repo::update_counts(db, link.series_id, *vol, *chap, *owned, now).await?;
        }
    }

    Ok(codex_link_repo::count(db).await? as usize)
}

/// Map a sweep error to `(auth_state, message)`. `auth_state` is `Some` only
/// for the two auth verdicts (401/403); other failures return `None` so the
/// caller records the error without overwriting a prior auth verdict.
fn classify_sweep_error(e: &CodexError) -> (Option<&'static str>, String) {
    match e {
        CodexError::Unauthorized => (
            Some(codex_status_repo::AUTH_UNAUTHORIZED),
            "api_key rejected (401)".to_string(),
        ),
        CodexError::Forbidden => (
            Some(codex_status_repo::AUTH_FORBIDDEN),
            "api_key lacks the series:read scope (403)".to_string(),
        ),
        other => (None, format!("external-index sweep failed: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{ActiveValue::Set, Database, EntityTrait};
    use td_codex::ExternalIdRef;
    use td_db::entities::series;

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    async fn insert_series(db: &DatabaseConnection, title: &str) -> i32 {
        series::Entity::insert(series::ActiveModel {
            canonical_title: Set(title.into()),
            metadata_source: Set("test".into()),
            metadata_fetched_at: Set(0),
            first_seen_at: Set(0),
            last_release_at: Set(0),
            owned: Set(0),
            ..Default::default()
        })
        .exec_with_returning(db)
        .await
        .unwrap()
        .id
    }

    fn item(uuid: &str, source: &str, ext_id: &str, vol: Option<f64>) -> ExternalIndexItem {
        ExternalIndexItem {
            id: uuid.into(),
            external_ids: vec![ExternalIdRef {
                source: source.into(),
                external_id: ext_id.into(),
                external_url: None,
            }],
            local_max_volume: vol,
            local_max_chapter: None,
            volumes_owned: vol.map(|v| v as i64),
        }
    }

    #[tokio::test]
    async fn apply_sweep_matches_external_id_and_creates_auto_link() {
        let db = fresh_db().await;
        let s = insert_series(&db, "Chainsaw Man").await;
        series_external_ids_repo::upsert(&db, s, "mangabaka", "12345", 0)
            .await
            .unwrap();

        // Codex namespaces its source as plugin:mangabaka; normalization maps it.
        let items = vec![item("codex-uuid-1", "plugin:mangabaka", "12345", Some(7.0))];
        let linked = apply_sweep(&db, &items, 1000).await.unwrap();
        assert_eq!(linked, 1);

        let link = codex_link_repo::get(&db, s).await.unwrap().unwrap();
        assert_eq!(link.codex_series_uuid, "codex-uuid-1");
        assert_eq!(link.link_kind, codex_link_repo::KIND_AUTO);
        assert_eq!(link.matched_provider.as_deref(), Some("mangabaka"));
        assert_eq!(link.local_max_volume, Some(7.0));
    }

    #[tokio::test]
    async fn apply_sweep_skips_unmatched_and_unmappable_sources() {
        let db = fresh_db().await;
        let s = insert_series(&db, "Owned").await;
        series_external_ids_repo::upsert(&db, s, "mangabaka", "111", 0)
            .await
            .unwrap();

        let items = vec![
            // Maps + matches -> linked.
            item("u-match", "api:mangabaka", "111", Some(1.0)),
            // Maps but no local series with that id -> no link.
            item("u-nomatch", "api:mangabaka", "999", Some(1.0)),
            // Unmappable source (comicinfo) -> ignored.
            item("u-comicinfo", "comicinfo", "abc", Some(1.0)),
        ];
        let linked = apply_sweep(&db, &items, 1).await.unwrap();
        assert_eq!(linked, 1, "only the matched series is linked");
        assert!(codex_link_repo::get(&db, s).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn apply_sweep_prunes_stale_auto_links() {
        let db = fresh_db().await;
        let s1 = insert_series(&db, "A").await;
        let s2 = insert_series(&db, "B").await;
        series_external_ids_repo::upsert(&db, s1, "mangabaka", "1", 0)
            .await
            .unwrap();
        series_external_ids_repo::upsert(&db, s2, "mangabaka", "2", 0)
            .await
            .unwrap();

        // First sweep links both.
        let items = vec![
            item("u1", "plugin:mangabaka", "1", Some(1.0)),
            item("u2", "plugin:mangabaka", "2", Some(1.0)),
        ];
        assert_eq!(apply_sweep(&db, &items, 1).await.unwrap(), 2);

        // Second sweep no longer includes s2's match -> its auto link is pruned.
        let items = vec![item("u1", "plugin:mangabaka", "1", Some(2.0))];
        assert_eq!(apply_sweep(&db, &items, 2).await.unwrap(), 1);
        assert!(codex_link_repo::get(&db, s1).await.unwrap().is_some());
        assert!(codex_link_repo::get(&db, s2).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn apply_sweep_refreshes_manual_link_counts_by_uuid() {
        let db = fresh_db().await;
        let s = insert_series(&db, "Manually linked").await;
        // Operator hand-links to a Codex uuid that has no matchable external id.
        codex_link_repo::upsert_manual(&db, s, "manual-uuid", 0)
            .await
            .unwrap();

        // The sweep includes that uuid (as its own series row, no external id
        // tsundoku knows). Counts should flow onto the manual link.
        let mut it = item("manual-uuid", "comicinfo", "x", Some(15.0));
        it.local_max_chapter = Some(160.0);
        it.volumes_owned = Some(15);
        apply_sweep(&db, &[it], 500).await.unwrap();

        let link = codex_link_repo::get(&db, s).await.unwrap().unwrap();
        assert_eq!(link.link_kind, codex_link_repo::KIND_MANUAL);
        assert_eq!(link.local_max_volume, Some(15.0));
        assert_eq!(link.local_max_chapter, Some(160.0));
        assert_eq!(link.volumes_owned, Some(15));
        assert_eq!(link.synced_at, 500);
    }

    #[test]
    fn classify_sweep_error_splits_auth_verdicts() {
        let (auth, msg) = classify_sweep_error(&CodexError::Unauthorized);
        assert_eq!(auth, Some(codex_status_repo::AUTH_UNAUTHORIZED));
        assert!(msg.contains("401"));

        let (auth, msg) = classify_sweep_error(&CodexError::Forbidden);
        assert_eq!(auth, Some(codex_status_repo::AUTH_FORBIDDEN));
        assert!(msg.contains("403"));

        // A non-auth failure does not assert an auth verdict.
        let (auth, _) = classify_sweep_error(&CodexError::Unexpected(500));
        assert_eq!(auth, None);
    }
}
