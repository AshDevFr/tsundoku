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

// Denormalized rating on `series`, populated from the active provider's
// payload on the next persist (MangaBaka's `rating` field, normalized
// from the dump's native 0-100 scale to 0-10 by the provider mapping).
// Carried as a real column for the same reason as `total_volumes` /
// `total_chapters`: keeps the candidate query a flat SELECT.
//
// Existing rows are left NULL until the next `refresh-metadata` cycle
// re-persists them (the offline-store row hash changes once `rating` is
// included in `SerializedRow`, so hash-skip won't block the backfill).
const UP: &[&str] = &["ALTER TABLE series ADD COLUMN rating REAL"];

const DOWN: &[&str] = &[
    // SQLite's ALTER TABLE DROP COLUMN landed in 3.35 but our minimum
    // bar is older; a partial down is a no-op.
];
