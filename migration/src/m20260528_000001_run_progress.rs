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

// Live-progress columns on every `*_runs` table. While a job is still in
// `status = 'running'`, the loop body throttles UPDATEs to these columns so
// the admin pill can render "47 / 75" without waiting for the SSE channel
// to replay. After finalize, the columns are a footnote (last reported
// state at crash time, if any).
//
// All nullable: jobs that don't report progress just leave them NULL, and
// existing pre-migration rows decode the same way.
const UP: &[&str] = &[
    "ALTER TABLE poll_runs ADD COLUMN progress_current INTEGER",
    "ALTER TABLE poll_runs ADD COLUMN progress_total INTEGER",
    "ALTER TABLE poll_runs ADD COLUMN progress_phase TEXT",
    "ALTER TABLE provider_refreshes ADD COLUMN progress_current INTEGER",
    "ALTER TABLE provider_refreshes ADD COLUMN progress_total INTEGER",
    "ALTER TABLE provider_refreshes ADD COLUMN progress_phase TEXT",
    "ALTER TABLE series_refresh_runs ADD COLUMN progress_current INTEGER",
    "ALTER TABLE series_refresh_runs ADD COLUMN progress_total INTEGER",
    "ALTER TABLE series_refresh_runs ADD COLUMN progress_phase TEXT",
];

const DOWN: &[&str] = &[
    // SQLite's ALTER TABLE DROP COLUMN landed in 3.35 but our minimum bar
    // is older; a partial down is a no-op.
];
