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

// A nullable description column on `series`, populated from the active
// provider's payload on the next persist. The feed UI surfaces it as a
// short clamped synopsis on the list view; the detail page shows it in
// full. We carry it as a real column rather than re-deriving it from
// `metadata_json` per request so the list endpoint stays a flat SELECT.
const UP: &[&str] = &["ALTER TABLE series ADD COLUMN description TEXT"];

const DOWN: &[&str] = &[
    // SQLite's ALTER TABLE DROP COLUMN landed in 3.35 but our minimum
    // bar is older; a partial down is a no-op.
];
