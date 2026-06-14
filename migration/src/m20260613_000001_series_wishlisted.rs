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

// Admin-only "wishlist" flag on a series. A single nullable timestamp does both
// jobs: `wishlisted_at IS NOT NULL` *is* the flag, and the value gives the
// wishlist view a "recently clipped" sort. Independent of `owned` — clipping a
// series the operator already owns is allowed, and import into Codex never
// clears it (manual removal only). The partial index keeps the wishlist page's
// `WHERE wishlisted_at IS NOT NULL ORDER BY wishlisted_at DESC` cheap.
const UP: &[&str] = &[
    "ALTER TABLE series ADD COLUMN wishlisted_at INTEGER",
    "CREATE INDEX idx_series_wishlisted_at ON series (wishlisted_at) WHERE wishlisted_at IS NOT NULL",
];

const DOWN: &[&str] = &[
    "DROP INDEX IF EXISTS idx_series_wishlisted_at",
    // SQLite's ALTER TABLE DROP COLUMN landed in 3.35 but our minimum bar is
    // older; the column drop is a no-op, matching the repo's convention.
];
