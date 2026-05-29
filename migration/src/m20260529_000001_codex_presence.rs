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

// Codex presence overlay. `codex_series_link` is one row per tsundoku series
// that maps to a Codex series — either matched automatically by a shared
// external id (`link_kind = 'auto'`) or hand-linked by the operator
// (`link_kind = 'manual'`). The PK is `series_id` (FK to series, cascade) so a
// deleted series drops its link automatically. `local_max_*` are Codex's
// highest owned volume/chapter (the comparison basis for the green/blue
// status); `volumes_owned` is a soft, display-only file count. A sweep only
// rewrites `auto` rows, so `manual` links survive a re-sync.
//
// `codex_status` is a single-row (`id = 1`) connection-health record so the
// admin UI can show reachability / version / auth state / last error instead
// of those living only in the logs.
const UP: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS codex_series_link (
        series_id           INTEGER PRIMARY KEY NOT NULL REFERENCES series(id) ON DELETE CASCADE,
        codex_series_uuid   TEXT NOT NULL,
        local_max_volume    REAL,
        local_max_chapter   REAL,
        volumes_owned       INTEGER,
        link_kind           TEXT NOT NULL,
        matched_provider    TEXT,
        matched_external_id TEXT,
        synced_at           INTEGER NOT NULL
    )",
    // The sweep deletes stale auto rows by `link_kind`; index it so that
    // delete and the auto-only rewrite don't table-scan.
    "CREATE INDEX IF NOT EXISTS idx_codex_series_link_kind ON codex_series_link (link_kind)",
    "CREATE TABLE IF NOT EXISTS codex_status (
        id                INTEGER PRIMARY KEY NOT NULL,
        codex_name        TEXT,
        codex_version     TEXT,
        reachable         INTEGER NOT NULL DEFAULT 0,
        auth_state        TEXT NOT NULL DEFAULT 'unknown',
        last_preflight_at INTEGER,
        last_success_at   INTEGER,
        last_error        TEXT,
        linked_count      INTEGER
    )",
];

const DOWN: &[&str] = &[
    "DROP TABLE IF EXISTS codex_status",
    "DROP TABLE IF EXISTS codex_series_link",
];
