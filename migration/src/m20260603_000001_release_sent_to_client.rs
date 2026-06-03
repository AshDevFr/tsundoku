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

// Two nullable columns recording that a release was pushed to the operator's
// torrent client: `sent_to_client_at` (epoch seconds) anchors the "Sent" badge
// and a record of when, `sent_to_client_label` keeps the label that was used.
// Both NULL means never sent.
const UP: &[&str] = &[
    "ALTER TABLE releases ADD COLUMN sent_to_client_at INTEGER",
    "ALTER TABLE releases ADD COLUMN sent_to_client_label TEXT",
];

const DOWN: &[&str] = &[
    // SQLite's ALTER TABLE DROP COLUMN landed in 3.35 but our minimum
    // bar is older; a partial down is a no-op.
];
