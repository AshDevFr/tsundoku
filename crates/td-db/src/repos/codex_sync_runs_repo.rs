//! Append-only per-sweep history for the Codex presence sync
//! (`codex_sync_runs`). Each sweep attempt, success or failure, records one row
//! here; the latest-snapshot columns stay on the singleton `codex_status` row.

use anyhow::Result;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, QueryOrder, QuerySelect, Set};

use crate::entities::codex_sync_runs;

pub use codex_sync_runs::{ActiveModel, Column, Entity, Model};

/// `outcome` discriminants. `Success` carries `fetched`/`linked` counts; the
/// failure variants carry an `error` message instead.
pub const OUTCOME_SUCCESS: &str = "success";
pub const OUTCOME_PREFLIGHT_FAILED: &str = "preflight_failed";
pub const OUTCOME_AUTH_FAILED: &str = "auth_failed";
pub const OUTCOME_ERROR: &str = "error";

/// Record one sweep attempt. `trigger` is `cron` | `manual`. On success pass
/// the `fetched`/`linked` counts and no error; on failure pass the error and
/// leave the counts `None`.
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    db: &DatabaseConnection,
    ran_at: i64,
    trigger: &str,
    outcome: &str,
    fetched_count: Option<i64>,
    linked_count: Option<i64>,
    error: Option<&str>,
) -> Result<()> {
    ActiveModel {
        ran_at: Set(ran_at),
        trigger: Set(trigger.to_string()),
        outcome: Set(outcome.to_string()),
        fetched_count: Set(fetched_count),
        linked_count: Set(linked_count),
        error: Set(error.map(str::to_string)),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(())
}

/// The most recent sweep attempts, newest first.
pub async fn list_recent(db: &DatabaseConnection, limit: u64) -> Result<Vec<Model>> {
    Ok(Entity::find()
        .order_by_desc(Column::Id)
        .limit(limit)
        .all(db)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::Database;

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    #[tokio::test]
    async fn records_success_and_failure_newest_first() {
        let db = fresh_db().await;

        insert(
            &db,
            100,
            "cron",
            OUTCOME_SUCCESS,
            Some(412),
            Some(130),
            None,
        )
        .await
        .unwrap();
        insert(
            &db,
            200,
            "manual",
            OUTCOME_AUTH_FAILED,
            None,
            None,
            Some("api_key rejected (401)"),
        )
        .await
        .unwrap();

        let recent = list_recent(&db, 10).await.unwrap();
        assert_eq!(recent.len(), 2);
        // Newest first.
        assert_eq!(recent[0].ran_at, 200);
        assert_eq!(recent[0].trigger, "manual");
        assert_eq!(recent[0].outcome, OUTCOME_AUTH_FAILED);
        assert_eq!(recent[0].error.as_deref(), Some("api_key rejected (401)"));
        assert!(recent[0].fetched_count.is_none());
        assert_eq!(recent[1].ran_at, 100);
        assert_eq!(recent[1].outcome, OUTCOME_SUCCESS);
        assert_eq!(recent[1].fetched_count, Some(412));
        assert_eq!(recent[1].linked_count, Some(130));
    }
}
