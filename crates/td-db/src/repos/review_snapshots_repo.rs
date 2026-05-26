//! Hourly snapshot of the review queue depth.
//!
//! Writes one row per scheduler tick of `snapshot_review_queue`; the
//! frontend reads recent rows to render the depth-over-time chart.

use anyhow::Result;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, QuerySelect, Set, Statement,
};
use serde::Serialize;

use crate::entities::{releases, review_queue_snapshots};

#[derive(Debug, Clone, Copy, Default)]
pub struct PendingBreakdown {
    pub unresolved: i64,
    pub ambiguous: i64,
    pub review_pending: i64,
}

impl PendingBreakdown {
    pub fn total(&self) -> i64 {
        self.unresolved + self.ambiguous + self.review_pending
    }
}

#[derive(Debug, Clone, Serialize, FromQueryResult)]
struct PendingCountRow {
    status: String,
    count: i64,
}

/// Tally pending review rows by status. The snapshot job calls this just
/// before inserting a row so the on-disk numbers reflect the same instant.
pub async fn pending_breakdown(db: &DatabaseConnection) -> Result<PendingBreakdown> {
    let sql = "SELECT resolution_status AS status, COUNT(*) AS count
               FROM releases
               WHERE resolution_status IN ('unresolved', 'ambiguous', 'review_pending')
               GROUP BY resolution_status";
    let stmt = Statement::from_sql_and_values(db.get_database_backend(), sql, []);
    let rows = PendingCountRow::find_by_statement(stmt).all(db).await?;
    let mut out = PendingBreakdown::default();
    for row in rows {
        match row.status.as_str() {
            "unresolved" => out.unresolved = row.count,
            "ambiguous" => out.ambiguous = row.count,
            "review_pending" => out.review_pending = row.count,
            _ => {}
        }
    }
    Ok(out)
}

/// Age (in seconds) of the longest-waiting pending release at `now`.
/// `None` when the queue is empty.
pub async fn oldest_pending_age_seconds(db: &DatabaseConnection, now: i64) -> Result<Option<i64>> {
    let rows = releases::Entity::find()
        .filter(releases::Column::ResolutionStatus.is_in([
            "unresolved",
            "ambiguous",
            "review_pending",
        ]))
        .order_by_asc(releases::Column::ObservedAt)
        .limit(1)
        .all(db)
        .await?;
    Ok(rows.into_iter().next().map(|r| now - r.observed_at))
}

pub async fn insert_snapshot(
    db: &DatabaseConnection,
    captured_at: i64,
    breakdown: PendingBreakdown,
    oldest_pending_seconds: Option<i64>,
) -> Result<i64> {
    let model = review_queue_snapshots::ActiveModel {
        captured_at: Set(captured_at),
        pending_count: Set(breakdown.total()),
        unresolved_count: Set(breakdown.unresolved),
        ambiguous_count: Set(breakdown.ambiguous),
        review_pending_count: Set(breakdown.review_pending),
        oldest_pending_seconds: Set(oldest_pending_seconds),
        ..Default::default()
    };
    let res = review_queue_snapshots::Entity::insert(model)
        .exec(db)
        .await?;
    Ok(res.last_insert_id)
}

pub async fn snapshots_between(
    db: &DatabaseConnection,
    since: i64,
    until: i64,
) -> Result<Vec<review_queue_snapshots::Model>> {
    Ok(review_queue_snapshots::Entity::find()
        .filter(review_queue_snapshots::Column::CapturedAt.gte(since))
        .filter(review_queue_snapshots::Column::CapturedAt.lt(until))
        .order_by_asc(review_queue_snapshots::Column::CapturedAt)
        .all(db)
        .await?)
}

#[derive(Debug, Clone, Serialize, FromQueryResult)]
#[serde(rename_all = "camelCase")]
pub struct TimeToDecisionRow {
    pub p50_seconds: Option<f64>,
    pub count: i64,
}

/// Median (P50) time-to-decision in seconds for releases resolved in
/// `[since, until)`. "Decision" = `resolved_at` is set (i.e. status went
/// to `resolved`).
pub async fn time_to_decision(
    db: &DatabaseConnection,
    since: i64,
    until: i64,
) -> Result<TimeToDecisionRow> {
    let sql = "WITH samples AS (
            SELECT (resolved_at - observed_at) AS ttr,
                   ROW_NUMBER() OVER (ORDER BY (resolved_at - observed_at) ASC) AS rn,
                   COUNT(*) OVER () AS total
            FROM releases
            WHERE resolved_at IS NOT NULL
              AND resolved_at >= ?1 AND resolved_at < ?2
        )
        SELECT
            (SELECT CAST(ttr AS REAL) FROM samples WHERE rn = CAST(0.5 * total + 0.5 AS INTEGER) LIMIT 1) AS p50_seconds,
            (SELECT COUNT(*) FROM samples) AS count";
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [since.into(), until.into()],
    );
    Ok(TimeToDecisionRow::find_by_statement(stmt)
        .one(db)
        .await?
        .unwrap_or(TimeToDecisionRow {
            p50_seconds: None,
            count: 0,
        }))
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
    async fn snapshot_insert_and_read_back() {
        let db = fresh_db().await;
        let id = insert_snapshot(
            &db,
            1_700_000_100,
            PendingBreakdown {
                unresolved: 3,
                ambiguous: 1,
                review_pending: 2,
            },
            Some(7_200),
        )
        .await
        .unwrap();
        assert!(id > 0);
        let rows = snapshots_between(&db, 0, i64::MAX).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pending_count, 6);
        assert_eq!(rows[0].oldest_pending_seconds, Some(7_200));
    }
}
