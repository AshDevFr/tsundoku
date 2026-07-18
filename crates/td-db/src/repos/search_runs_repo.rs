//! Append-only per-run audit for per-series release searches
//! (`search_runs`). A row is inserted as `running` when a walk starts and
//! completed in place with its counts; the series page polls the newest
//! row for liveness, so `running` is a live state, not just history.

use anyhow::Result;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};

use crate::entities::search_runs;

pub use search_runs::{ActiveModel, Column, Entity, Model};

/// `outcome` discriminants. `Running` rows are live; a `Running` row that
/// survives a process restart is dead and gets flipped to `Error` at boot
/// by [`mark_stale_running_interrupted`].
pub const OUTCOME_RUNNING: &str = "running";
pub const OUTCOME_SUCCESS: &str = "success";
pub const OUTCOME_ERROR: &str = "error";

/// Trigger discriminants: the admin button vs the one-shot CLI.
pub const TRIGGER_MANUAL: &str = "manual";
pub const TRIGGER_CLI: &str = "cli";

/// Completion counts, all measured across the whole run.
#[derive(Debug, Clone, Copy, Default)]
pub struct SearchRunCounts {
    pub queries_attempted: i64,
    pub pages_fetched: i64,
    /// Hits returned by the upstream across all pages (pre-dedup).
    pub releases_seen: i64,
    /// Releases that were actually new to the catalog.
    pub releases_new: i64,
}

/// Insert the `running` row for a starting walk; returns its id for
/// [`complete`].
pub async fn insert_running(
    db: &DatabaseConnection,
    ran_at: i64,
    search_name: &str,
    series_id: i32,
    trigger: &str,
) -> Result<i64> {
    let row = ActiveModel {
        ran_at: Set(ran_at),
        search_name: Set(search_name.to_string()),
        series_id: Set(series_id),
        trigger: Set(trigger.to_string()),
        outcome: Set(OUTCOME_RUNNING.to_string()),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(row.id)
}

/// Complete a run in place: `success` with counts, or `error` with the
/// message (counts still recorded so a partial walk's work is visible).
pub async fn complete(
    db: &DatabaseConnection,
    id: i64,
    finished_at: i64,
    outcome: &str,
    counts: SearchRunCounts,
    error: Option<&str>,
) -> Result<()> {
    ActiveModel {
        id: Set(id),
        finished_at: Set(Some(finished_at)),
        outcome: Set(outcome.to_string()),
        queries_attempted: Set(Some(counts.queries_attempted)),
        pages_fetched: Set(Some(counts.pages_fetched)),
        releases_seen: Set(Some(counts.releases_seen)),
        releases_new: Set(Some(counts.releases_new)),
        error: Set(error.map(str::to_string)),
        ..Default::default()
    }
    .update(db)
    .await?;
    Ok(())
}

/// The most recent runs for one series, newest first.
pub async fn recent_for_series(
    db: &DatabaseConnection,
    series_id: i32,
    limit: u64,
) -> Result<Vec<Model>> {
    Ok(Entity::find()
        .filter(Column::SeriesId.eq(series_id))
        .order_by_desc(Column::Id)
        .limit(limit)
        .all(db)
        .await?)
}

/// Boot reconciliation: rows left `running` by a killed process would poll
/// as live forever, so flip them to `error` with an explanatory message.
/// Returns how many rows were flipped.
pub async fn mark_stale_running_interrupted(db: &DatabaseConnection, now: i64) -> Result<u64> {
    let res = Entity::update_many()
        .col_expr(Column::Outcome, OUTCOME_ERROR.into())
        .col_expr(Column::FinishedAt, now.into())
        .col_expr(Column::Error, "interrupted (process exited mid-run)".into())
        .filter(Column::Outcome.eq(OUTCOME_RUNNING))
        .exec(db)
        .await?;
    Ok(res.rows_affected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::series;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{ActiveValue::Set as AvSet, ConnectionTrait, Database, ModelTrait};

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        // In-memory test DBs don't run the pool's per-connection PRAGMAs.
        db.execute_unprepared("PRAGMA foreign_keys = ON")
            .await
            .unwrap();
        db
    }

    async fn insert_series(db: &DatabaseConnection, title: &str) -> i32 {
        let model = series::ActiveModel {
            canonical_title: AvSet(title.into()),
            metadata_source: AvSet("test".into()),
            metadata_fetched_at: AvSet(0),
            first_seen_at: AvSet(0),
            last_release_at: AvSet(0),
            owned: AvSet(0),
            ..Default::default()
        };
        series::Entity::insert(model)
            .exec_with_returning(db)
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn run_lifecycle_running_to_success() {
        let db = fresh_db().await;
        let series_id = insert_series(&db, "Solo Leveling").await;

        let id = insert_running(&db, 100, "Nyaa Eng", series_id, TRIGGER_MANUAL)
            .await
            .unwrap();
        let rows = recent_for_series(&db, series_id, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, OUTCOME_RUNNING);
        assert!(rows[0].finished_at.is_none());
        assert!(rows[0].releases_new.is_none());

        complete(
            &db,
            id,
            160,
            OUTCOME_SUCCESS,
            SearchRunCounts {
                queries_attempted: 4,
                pages_fetched: 6,
                releases_seen: 120,
                releases_new: 7,
            },
            None,
        )
        .await
        .unwrap();

        let rows = recent_for_series(&db, series_id, 10).await.unwrap();
        assert_eq!(rows[0].outcome, OUTCOME_SUCCESS);
        assert_eq!(rows[0].finished_at, Some(160));
        assert_eq!(rows[0].queries_attempted, Some(4));
        assert_eq!(rows[0].pages_fetched, Some(6));
        assert_eq!(rows[0].releases_seen, Some(120));
        assert_eq!(rows[0].releases_new, Some(7));
        assert!(rows[0].error.is_none());
    }

    #[tokio::test]
    async fn error_completion_keeps_partial_counts() {
        let db = fresh_db().await;
        let series_id = insert_series(&db, "Frieren").await;
        let id = insert_running(&db, 100, "Nyaa Raw", series_id, TRIGGER_CLI)
            .await
            .unwrap();
        complete(
            &db,
            id,
            130,
            OUTCOME_ERROR,
            SearchRunCounts {
                queries_attempted: 2,
                pages_fetched: 1,
                releases_seen: 40,
                releases_new: 3,
            },
            Some("nyaa unreachable"),
        )
        .await
        .unwrap();
        let rows = recent_for_series(&db, series_id, 10).await.unwrap();
        assert_eq!(rows[0].outcome, OUTCOME_ERROR);
        assert_eq!(rows[0].error.as_deref(), Some("nyaa unreachable"));
        assert_eq!(rows[0].releases_new, Some(3));
    }

    #[tokio::test]
    async fn recent_is_scoped_to_the_series_and_newest_first() {
        let db = fresh_db().await;
        let a = insert_series(&db, "A").await;
        let b = insert_series(&db, "B").await;
        insert_running(&db, 100, "eng", a, TRIGGER_MANUAL)
            .await
            .unwrap();
        insert_running(&db, 200, "raw", a, TRIGGER_MANUAL)
            .await
            .unwrap();
        insert_running(&db, 300, "eng", b, TRIGGER_MANUAL)
            .await
            .unwrap();

        let rows = recent_for_series(&db, a, 10).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].ran_at, 200);
        assert_eq!(rows[1].ran_at, 100);
    }

    #[tokio::test]
    async fn deleting_the_series_cascades_its_runs() {
        let db = fresh_db().await;
        let series_id = insert_series(&db, "Gone").await;
        insert_running(&db, 100, "eng", series_id, TRIGGER_MANUAL)
            .await
            .unwrap();

        let series_row = series::Entity::find_by_id(series_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        series_row.delete(&db).await.unwrap();

        assert!(
            recent_for_series(&db, series_id, 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn boot_reconciliation_flips_only_running_rows() {
        let db = fresh_db().await;
        let series_id = insert_series(&db, "S").await;
        let stale = insert_running(&db, 100, "eng", series_id, TRIGGER_MANUAL)
            .await
            .unwrap();
        let done = insert_running(&db, 200, "eng", series_id, TRIGGER_MANUAL)
            .await
            .unwrap();
        complete(
            &db,
            done,
            260,
            OUTCOME_SUCCESS,
            SearchRunCounts::default(),
            None,
        )
        .await
        .unwrap();

        let flipped = mark_stale_running_interrupted(&db, 999).await.unwrap();
        assert_eq!(flipped, 1);

        let rows = recent_for_series(&db, series_id, 10).await.unwrap();
        let stale_row = rows.iter().find(|r| r.id == stale).unwrap();
        assert_eq!(stale_row.outcome, OUTCOME_ERROR);
        assert_eq!(stale_row.finished_at, Some(999));
        assert!(stale_row.error.as_deref().unwrap().contains("interrupted"));
        let done_row = rows.iter().find(|r| r.id == done).unwrap();
        assert_eq!(done_row.outcome, OUTCOME_SUCCESS);
    }
}
