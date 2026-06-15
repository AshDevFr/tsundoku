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

// Official publication start/end dates from the active provider's payload
// (MangaBaka's `published_start_date` / `published_end_date`, ISO `YYYY-MM-DD`
// strings). Stored as TEXT so `ORDER BY` sorts them lexicographically without
// any date parsing; the feed's "Publication date" sort uses the start date.
// Distinct from `last_release_at`, which tracks the last *discovered* release.
//
// Existing rows stay NULL until the next refresh cycle (cron, the
// `POST /api/v1/series/refresh-all` endpoint, or `tsundoku refresh-series`)
// re-persists them. The offline-store row hash changes once these are included
// in `SerializedRow`, so hash-skip won't block the backfill.
const UP: &[&str] = &[
    "ALTER TABLE series ADD COLUMN published_start_date TEXT",
    "ALTER TABLE series ADD COLUMN published_end_date TEXT",
];

const DOWN: &[&str] = &[
    // SQLite's ALTER TABLE DROP COLUMN landed in 3.35 but our minimum
    // bar is older; a partial down is a no-op.
];
