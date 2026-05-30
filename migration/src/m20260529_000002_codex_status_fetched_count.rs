use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE codex_status ADD COLUMN fetched_count INTEGER")
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite DROP COLUMN is gated on a newer minimum than we target; the
        // partial down is a no-op, consistent with the other migrations.
        Ok(())
    }
}

// `fetched_count` records how many series the last successful sweep pulled from
// Codex (distinct from `linked_count`, the subset that matched a tsundoku
// series). Surfaced in the admin status panel so "9 of 412 linked" reads
// honestly.
