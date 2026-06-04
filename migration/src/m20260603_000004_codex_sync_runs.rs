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

// Per-sweep history for the Codex presence sync. `codex_status` keeps only the
// last sweep's snapshot; this append-only table records one row per sweep
// attempt (cron or manual) so the admin UI can show a timeline of refreshes,
// the counts each one produced, and why a sweep failed. `outcome` is
// `success` | `preflight_failed` | `auth_failed` | `error`; `fetched_count` /
// `linked_count` are set only on `success`, `error` only otherwise.
const UP: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS codex_sync_runs (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        ran_at        INTEGER NOT NULL,
        trigger       TEXT NOT NULL,
        outcome       TEXT NOT NULL,
        fetched_count INTEGER,
        linked_count  INTEGER,
        error         TEXT
    )",
    "CREATE INDEX IF NOT EXISTS idx_codex_sync_runs_ran_at ON codex_sync_runs (ran_at)",
];

const DOWN: &[&str] = &["DROP TABLE IF EXISTS codex_sync_runs"];
