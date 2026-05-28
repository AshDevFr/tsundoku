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

// Denormalized display counts on `series`, populated from the active
// provider's payload on the next persist (MangaBaka's `final_volume` and
// `total_chapters`). The review UI shows them on each candidate so the
// operator can compare a release's contents against the series' length.
// Carried as real columns (like `cover_url`, `year`) rather than re-derived
// from `metadata_json` per request so the candidate query stays a flat SELECT.
// These differ from `highest_volume` / `highest_chapter`, which track the
// highest span seen across observed releases, not the series' published total.
const UP: &[&str] = &[
    "ALTER TABLE series ADD COLUMN total_volumes INTEGER",
    "ALTER TABLE series ADD COLUMN total_chapters INTEGER",
];

const DOWN: &[&str] = &[
    // SQLite's ALTER TABLE DROP COLUMN landed in 3.35 but our minimum
    // bar is older; a partial down is a no-op.
];
