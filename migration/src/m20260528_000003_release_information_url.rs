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

// A nullable column holding the URL from a release's "Information" field,
// verbatim. Unlike `extracted_links_json` (only provider links we resolve
// against), this keeps any cited reference — a publisher page, a Discord
// invite — so the review UI can surface the uploader's source. Backfilled
// on the next detail-fetch poll for rows persisted before this shipped.
const UP: &[&str] = &["ALTER TABLE releases ADD COLUMN information_url TEXT"];

const DOWN: &[&str] = &[
    // SQLite's ALTER TABLE DROP COLUMN landed in 3.35 but our minimum
    // bar is older; a partial down is a no-op.
];
