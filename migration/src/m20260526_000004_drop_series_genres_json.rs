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

// Drops the legacy `series.genres_json` denormalized blob. The canonical
// source for a series' genres is now the `genres` + `series_genres` pair
// (created in the genres_tags migration); the JSON column was kept as a
// short-lived fallback for a single release and is no longer read.
const UP: &[&str] = &["ALTER TABLE series DROP COLUMN genres_json"];

const DOWN: &[&str] = &["ALTER TABLE series ADD COLUMN genres_json TEXT"];
