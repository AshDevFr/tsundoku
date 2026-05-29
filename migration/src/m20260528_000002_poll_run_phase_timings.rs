use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for stmt in UP {
            db.execute_unprepared(stmt).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for stmt in DOWN {
            db.execute_unprepared(stmt).await?;
        }
        Ok(())
    }
}

// Per-phase wall-clock totals on `poll_runs`. `fetch_duration_ms` already
// times the source's listing fetch; these split out the two heavier phases
// that follow so the admin metrics card can answer "is this run slow
// because of HTTP enrich or because of the resolver?". Both nullable: pre-
// migration rows decode the same way, and skipped / errored ticks can
// leave them NULL without surprising the SUM aggregations (COALESCE 0).
const UP: &[&str] = &[
    "ALTER TABLE poll_runs ADD COLUMN enrich_duration_ms INTEGER",
    "ALTER TABLE poll_runs ADD COLUMN resolve_duration_ms INTEGER",
];

const DOWN: &[&str] = &[
    // SQLite ALTER TABLE DROP COLUMN is post-3.35; the project's minimum bar
    // is older, so a partial down is a no-op.
];
