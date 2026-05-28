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

// Per-tick history for the bulk series-metadata refresh job. Mirrors the
// shape of `provider_refreshes` (each tick is provider-scoped) but with
// per-tick counters tailored to "we walked N stale series and refreshed
// some of them" — which doesn't fit the cache-download semantics of the
// existing table. Keeping it separate also lets the admin UI render the
// two operations distinctly.
//
// `considered` is the size of the batch the selection query returned;
// `refreshed` + `unchanged` + `not_found` + `errored` partition that
// (modulo an early-break on a provider transport error, which leaves the
// remainder as zero on each counter). All counts are nullable so a row
// finalised at `running → failure` before the batch even started doesn't
// have to lie with zeros.
const UP: &[&str] = &[
    "CREATE TABLE series_refresh_runs (
        id                  INTEGER PRIMARY KEY AUTOINCREMENT,
        provider_id         TEXT NOT NULL,
        started_at          INTEGER NOT NULL,
        finished_at         INTEGER,
        status              TEXT NOT NULL,
        trigger             TEXT NOT NULL,
        considered_count    INTEGER,
        refreshed_count     INTEGER,
        unchanged_count     INTEGER,
        not_found_count     INTEGER,
        errored_count       INTEGER,
        fetch_duration_ms   INTEGER,
        error_message       TEXT,
        error_kind          TEXT
    )",
    "CREATE INDEX ix_series_refresh_runs_provider_started
        ON series_refresh_runs(provider_id, started_at DESC)",
    "CREATE INDEX ix_series_refresh_runs_started
        ON series_refresh_runs(started_at DESC)",
];

const DOWN: &[&str] = &[
    "DROP INDEX IF EXISTS ix_series_refresh_runs_started",
    "DROP INDEX IF EXISTS ix_series_refresh_runs_provider_started",
    "DROP TABLE IF EXISTS series_refresh_runs",
];
