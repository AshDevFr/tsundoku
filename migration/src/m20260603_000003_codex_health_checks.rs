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

// Codex reachability history, the same shape as `download_health_checks`: the
// `codex_status` singleton keeps the current snapshot, and a row is appended
// here only when reachability changes or on a manual test, giving the admin UI
// an uptime timeline without per-tick noise.
const UP: &[&str] = &["CREATE TABLE IF NOT EXISTS codex_health_checks (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        checked_at INTEGER NOT NULL,
        reachable  INTEGER NOT NULL,
        error      TEXT,
        trigger    TEXT NOT NULL
    )"];

const DOWN: &[&str] = &["DROP TABLE IF EXISTS codex_health_checks"];
