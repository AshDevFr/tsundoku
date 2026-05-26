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

// One row per unique MangaUpdates legacy numeric ID we've encountered.
// `modern_id IS NULL` is a tombstone: MU's redirect did not resolve to a
// `/series/{slug}/` path (the ID was retired). The reverse index helps
// when an operator wants to know which legacy IDs point at a modern slug.
const UP: &[&str] = &[
    "CREATE TABLE mangaupdates_id_map (
        legacy_id    INTEGER PRIMARY KEY,
        modern_id    TEXT,
        resolved_at  INTEGER NOT NULL
    )",
    "CREATE INDEX ix_mu_id_map_modern
        ON mangaupdates_id_map(modern_id)
        WHERE modern_id IS NOT NULL",
];

const DOWN: &[&str] = &[
    "DROP INDEX IF EXISTS ix_mu_id_map_modern",
    "DROP TABLE IF EXISTS mangaupdates_id_map",
];
