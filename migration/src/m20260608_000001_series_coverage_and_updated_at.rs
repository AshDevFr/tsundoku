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

// Per-series merged volume/chapter coverage plus a semantic change timestamp.
//
// `volume_coverage_json` / `chapter_coverage_json` hold the gap-preserving
// union (a JSON array of `{start,end}`) of every linked release's spans, NULL
// when a series has no parsed coverage. `updated_at` (epoch seconds) is bumped
// only when that coverage or the `highest_*` marks actually change — never on a
// metadata refresh — so it drives the "what changed since last poll" feed. The
// `(updated_at, id)` index backs the keyset cursor walk. Existing rows default
// to `updated_at = 0` and are populated by the `recompute-spans` backfill.
const UP: &[&str] = &[
    "ALTER TABLE series ADD COLUMN volume_coverage_json TEXT",
    "ALTER TABLE series ADD COLUMN chapter_coverage_json TEXT",
    "ALTER TABLE series ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0",
    "CREATE INDEX idx_series_updated_at_id ON series (updated_at, id)",
];

const DOWN: &[&str] = &[
    "DROP INDEX IF EXISTS idx_series_updated_at_id",
    // SQLite's ALTER TABLE DROP COLUMN landed in 3.35 but our minimum bar is
    // older; the column drops are a no-op, matching the repo's convention.
];
