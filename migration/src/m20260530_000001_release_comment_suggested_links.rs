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

// A nullable column holding external provider links found in a release's
// *comments* (serialized `ExternalLinks` JSON). These are untrusted — anyone
// can comment — so they never feed the resolver; the review UI surfaces them
// as operator-confirmable suggestions, kept separate from `extracted_links_json`
// (the uploader's links). Backfilled on the next detail-fetch poll.
const UP: &[&str] = &["ALTER TABLE releases ADD COLUMN comment_suggested_links_json TEXT"];

const DOWN: &[&str] = &[
    // SQLite's ALTER TABLE DROP COLUMN landed in 3.35 but our minimum
    // bar is older; a partial down is a no-op.
];
