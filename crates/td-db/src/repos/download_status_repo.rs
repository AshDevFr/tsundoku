//! Download-client connection health: a single-row snapshot
//! (`download_status`, `id = 1`) plus an append-only history
//! (`download_health_checks`).
//!
//! [`record_check`] rewrites the snapshot on *every* probe but appends to the
//! history only when reachability changes or the check was triggered manually,
//! so a frequent health cron leaves a readable transition timeline rather than
//! one row per tick. The pool is pinned to one connection, so the
//! read-then-write needed to detect a transition is race-free here.

use anyhow::Result;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, QueryOrder, QuerySelect, Set};

use crate::entities::{download_health_checks, download_status};

pub use download_status::{ActiveModel, Column, Entity, Model};

use super::TRIGGER_MANUAL;

/// The fixed primary key of the singleton snapshot row.
const ROW_ID: i32 = 1;

/// The current status snapshot, or `None` if no probe has run yet.
pub async fn get(db: &DatabaseConnection) -> Result<Option<Model>> {
    Ok(Entity::find_by_id(ROW_ID).one(db).await?)
}

/// Record the outcome of a connection probe. Always upserts the snapshot
/// (`reachable`, `last_test_at`, `last_error`); stamps `last_change_at` and
/// appends a `download_health_checks` row only when reachability *transitions*
/// (or the very first probe) — plus always appends when `trigger` is manual, so
/// an operator's explicit test is always recorded. Returns whether reachability
/// transitioned.
pub async fn record_check(
    db: &DatabaseConnection,
    reachable: bool,
    error: Option<&str>,
    at: i64,
    trigger: &str,
) -> Result<bool> {
    let previous = get(db).await?.map(|m| m.reachable);
    let transitioned = previous != Some(reachable);

    let mut snapshot = ActiveModel {
        id: Set(ROW_ID),
        reachable: Set(reachable),
        last_test_at: Set(Some(at)),
        last_error: Set(error.map(str::to_string)),
        ..Default::default()
    };
    let mut update_columns = vec![Column::Reachable, Column::LastTestAt, Column::LastError];
    if transitioned {
        snapshot.last_change_at = Set(Some(at));
        update_columns.push(Column::LastChangeAt);
    }
    Entity::insert(snapshot)
        .on_conflict(
            OnConflict::column(Column::Id)
                .update_columns(update_columns)
                .to_owned(),
        )
        .exec(db)
        .await?;

    if transitioned || trigger == TRIGGER_MANUAL {
        download_health_checks::ActiveModel {
            checked_at: Set(at),
            reachable: Set(reachable),
            error: Set(error.map(str::to_string)),
            trigger: Set(trigger.to_string()),
            ..Default::default()
        }
        .insert(db)
        .await?;
    }

    Ok(transitioned)
}

/// The most recent health-check rows, newest first.
pub async fn list_recent_checks(
    db: &DatabaseConnection,
    limit: u64,
) -> Result<Vec<download_health_checks::Model>> {
    Ok(download_health_checks::Entity::find()
        .order_by_desc(download_health_checks::Column::Id)
        .limit(limit)
        .all(db)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::super::{TRIGGER_CRON, TRIGGER_LAUNCH, TRIGGER_MANUAL};
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::Database;

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    async fn check_count(db: &DatabaseConnection) -> usize {
        list_recent_checks(db, 100).await.unwrap().len()
    }

    #[tokio::test]
    async fn get_is_none_before_any_probe() {
        let db = fresh_db().await;
        assert!(get(&db).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn first_probe_is_a_transition_and_appends() {
        let db = fresh_db().await;
        let transitioned = record_check(&db, true, None, 100, TRIGGER_LAUNCH)
            .await
            .unwrap();
        assert!(transitioned, "first probe counts as a transition");

        let snap = get(&db).await.unwrap().unwrap();
        assert_eq!(snap.id, ROW_ID);
        assert!(snap.reachable);
        assert_eq!(snap.last_test_at, Some(100));
        assert_eq!(snap.last_change_at, Some(100));
        assert_eq!(check_count(&db).await, 1);
    }

    #[tokio::test]
    async fn steady_state_cron_updates_snapshot_without_appending() {
        let db = fresh_db().await;
        record_check(&db, true, None, 100, TRIGGER_LAUNCH)
            .await
            .unwrap();

        // Two more reachable cron probes: snapshot's last_test_at advances, but
        // no history rows are added and last_change_at stays put.
        assert!(
            !record_check(&db, true, None, 200, TRIGGER_CRON)
                .await
                .unwrap()
        );
        assert!(
            !record_check(&db, true, None, 300, TRIGGER_CRON)
                .await
                .unwrap()
        );

        let snap = get(&db).await.unwrap().unwrap();
        assert_eq!(snap.last_test_at, Some(300), "snapshot freshness advances");
        assert_eq!(snap.last_change_at, Some(100), "no transition since first");
        assert_eq!(check_count(&db).await, 1, "no extra history rows");
    }

    #[tokio::test]
    async fn a_flip_appends_a_history_row() {
        let db = fresh_db().await;
        record_check(&db, true, None, 100, TRIGGER_LAUNCH)
            .await
            .unwrap();

        let transitioned = record_check(&db, false, Some("connection refused"), 200, TRIGGER_CRON)
            .await
            .unwrap();
        assert!(transitioned);

        let snap = get(&db).await.unwrap().unwrap();
        assert!(!snap.reachable);
        assert_eq!(snap.last_error.as_deref(), Some("connection refused"));
        assert_eq!(
            snap.last_change_at,
            Some(200),
            "change time updated on flip"
        );
        assert_eq!(check_count(&db).await, 2);
    }

    #[tokio::test]
    async fn manual_probe_always_appends_even_without_a_flip() {
        let db = fresh_db().await;
        record_check(&db, true, None, 100, TRIGGER_LAUNCH)
            .await
            .unwrap();

        // Same state, but a manual test should still leave a record.
        assert!(
            !record_check(&db, true, None, 200, TRIGGER_MANUAL)
                .await
                .unwrap()
        );
        assert_eq!(check_count(&db).await, 2);

        // Newest-first ordering.
        let recent = list_recent_checks(&db, 10).await.unwrap();
        assert_eq!(recent[0].trigger, TRIGGER_MANUAL);
        assert_eq!(recent[0].checked_at, 200);
    }

    #[tokio::test]
    async fn snapshot_stays_a_singleton() {
        let db = fresh_db().await;
        for (i, reachable) in [true, false, true, false].into_iter().enumerate() {
            record_check(&db, reachable, None, i as i64, TRIGGER_CRON)
                .await
                .unwrap();
        }
        assert_eq!(Entity::find().all(&db).await.unwrap().len(), 1);
    }
}
