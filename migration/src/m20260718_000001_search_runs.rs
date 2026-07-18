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

// Append-only audit for per-series release searches, one row per run
// attempt (button or CLI), in the `codex_sync_runs` mold. A row is
// inserted as `outcome = 'running'` when the walk starts and completed in
// place (`success` | `error`) with its counts, so the series page can poll
// the newest row for liveness. Rows left `running` by a killed process are
// marked `error` at boot. `search_name` is the `[[search]]` entry name;
// entries are config-only, so the column is informational rather than a
// foreign key.
const UP: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS search_runs (
        id                INTEGER PRIMARY KEY AUTOINCREMENT,
        ran_at            INTEGER NOT NULL,
        finished_at       INTEGER,
        search_name       TEXT NOT NULL,
        series_id         INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
        trigger           TEXT NOT NULL,
        outcome           TEXT NOT NULL,
        queries_attempted INTEGER,
        pages_fetched     INTEGER,
        releases_seen     INTEGER,
        releases_new      INTEGER,
        error             TEXT
    )",
    "CREATE INDEX IF NOT EXISTS idx_search_runs_series ON search_runs (series_id, ran_at DESC)",
];

const DOWN: &[&str] = &["DROP TABLE IF EXISTS search_runs"];
