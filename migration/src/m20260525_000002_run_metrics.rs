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

// Per-tick history for the scheduler. `poll_runs` is one row per poll
// attempt (cron tick OR manual trigger); `provider_refreshes` mirrors the
// shape for cache refreshes. Status starts at "running" when the tick
// begins and gets updated to success / failure / skipped at the end.
// Timestamps are unix seconds, matching the rest of the schema.
const UP: &[&str] = &[
    "CREATE TABLE poll_runs (
        id              INTEGER PRIMARY KEY AUTOINCREMENT,
        source_name     TEXT NOT NULL,
        source_kind     TEXT NOT NULL,
        started_at      INTEGER NOT NULL,
        finished_at     INTEGER,
        status          TEXT NOT NULL,
        fetched_count   INTEGER,
        new_count       INTEGER,
        resolved_count  INTEGER,
        error_message   TEXT,
        trigger         TEXT NOT NULL
    )",
    "CREATE INDEX ix_poll_runs_source_started
        ON poll_runs(source_name, started_at DESC)",
    "CREATE INDEX ix_poll_runs_started ON poll_runs(started_at DESC)",
    "CREATE TABLE provider_refreshes (
        id              INTEGER PRIMARY KEY AUTOINCREMENT,
        provider_id     TEXT NOT NULL,
        started_at      INTEGER NOT NULL,
        finished_at     INTEGER,
        status          TEXT NOT NULL,
        bytes_downloaded INTEGER,
        record_count    INTEGER,
        error_message   TEXT,
        trigger         TEXT NOT NULL
    )",
    "CREATE INDEX ix_provider_refreshes_provider_started
        ON provider_refreshes(provider_id, started_at DESC)",
    "CREATE INDEX ix_provider_refreshes_started
        ON provider_refreshes(started_at DESC)",
];

const DOWN: &[&str] = &[
    "DROP INDEX IF EXISTS ix_provider_refreshes_started",
    "DROP INDEX IF EXISTS ix_provider_refreshes_provider_started",
    "DROP TABLE IF EXISTS provider_refreshes",
    "DROP INDEX IF EXISTS ix_poll_runs_started",
    "DROP INDEX IF EXISTS ix_poll_runs_source_started",
    "DROP TABLE IF EXISTS poll_runs",
];
