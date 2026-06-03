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

// Download-client observability, mirroring the codex_status pattern: a current
// state snapshot plus an append-only history, and a send audit log.
//
// `download_status` is a single-row (`id = 1`) connection-health snapshot,
// rewritten on every probe so the admin UI can show reachability / last-test /
// last-error. `download_health_checks` is the bounded history: a row is
// appended only when reachability *changes* or on a manual test, so an
// every-minute health cron doesn't spam it. `download_sends` audits every send
// attempt (including failures, which previously vanished into a 502) so the
// operator has a record of what was pushed and what bounced.
const UP: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS download_status (
        id             INTEGER PRIMARY KEY NOT NULL,
        reachable      INTEGER NOT NULL DEFAULT 0,
        last_test_at   INTEGER,
        last_change_at INTEGER,
        last_error     TEXT
    )",
    "CREATE TABLE IF NOT EXISTS download_health_checks (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        checked_at INTEGER NOT NULL,
        reachable  INTEGER NOT NULL,
        error      TEXT,
        trigger    TEXT NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS download_sends (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        release_id TEXT NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
        sent_at    INTEGER NOT NULL,
        label      TEXT,
        source     TEXT NOT NULL,
        success    INTEGER NOT NULL,
        error      TEXT
    )",
    // Audit rows are listed newest-first per release for the card detail view.
    "CREATE INDEX IF NOT EXISTS idx_download_sends_release ON download_sends (release_id)",
];

const DOWN: &[&str] = &[
    "DROP TABLE IF EXISTS download_sends",
    "DROP TABLE IF EXISTS download_health_checks",
    "DROP TABLE IF EXISTS download_status",
];
