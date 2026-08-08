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

// Which feeds carry a release — a many-to-many fact that was previously
// squeezed into the scalar `releases.source_name`.
//
// One upstream post legitimately appears in several configured feeds at once
// (an uploader feed and a query feed both matching it), and `releases` dedups
// on `(source_kind, external_id)`, which is correct: one post, one row. But
// the upsert never updates `source_name`, so the column records only whichever
// feed happened to write first.
//
// That made the poll's dedup hint wrong for every other feed carrying the
// post: it asks "which external_ids have I seen?" keyed on `source_name`, got
// no rows, and treated the post as new on every tick — re-fetching its detail
// page and re-running the whole resolver, forever. Observed in production at
// up to 471 resolution attempts on a single release, with per-feed averages of
// ~50 on the most heavily overlapping feeds.
//
// `releases.source_name` is deliberately kept as "first discovered by": it is
// still meaningful provenance, and keeping it avoids churning every DTO in the
// same change.
//
// The backfill can only seed what we know — one row per release, from the
// scalar. The full carrier set converges after one poll cycle.
const UP: &[&str] = &[
    "CREATE TABLE release_sources (
         release_id    TEXT    NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
         source_kind   TEXT    NOT NULL,
         source_name   TEXT    NOT NULL,
         first_seen_at INTEGER NOT NULL,
         PRIMARY KEY (release_id, source_name)
     )",
    // Drives the poll dedup hint: `WHERE source_kind = ? AND source_name = ?`.
    "CREATE INDEX idx_release_sources_lookup
         ON release_sources (source_kind, source_name)",
    "INSERT INTO release_sources (release_id, source_kind, source_name, first_seen_at)
         SELECT id, source_kind, source_name, observed_at FROM releases",
];

const DOWN: &[&str] = &[
    "DROP INDEX IF EXISTS idx_release_sources_lookup",
    "DROP TABLE IF EXISTS release_sources",
];
