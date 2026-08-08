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

// When tsundoku last *discovered* a release for this series, as opposed to
// `last_release_at`, which is the upstream post date of the newest linked
// release. The two diverge whenever a source surfaces an old post — a query
// feed pulling back-catalogue, a backfill, or the per-series release search —
// and the gap is routinely months to years. Sorting a discovery feed on
// `last_release_at` therefore buries brand-new finds among year-old rows.
//
// NULL means "nothing linked yet", which the nullable-aware sort sinks to the
// end in both directions — the correct placement for a series with no
// discovered releases.
//
// Backfilled from the linked releases' `observed_at`, and maintained from then
// on by `releases_repo::recompute_series_coverage`, alongside the other
// release-derived aggregates.
const UP: &[&str] = &[
    "ALTER TABLE series ADD COLUMN last_discovered_at INTEGER",
    "UPDATE series SET last_discovered_at = (
         SELECT MAX(r.observed_at) FROM releases r WHERE r.series_id = series.id
     )",
    "CREATE INDEX idx_series_last_discovered_at ON series (last_discovered_at)",
];

const DOWN: &[&str] = &[
    "DROP INDEX IF EXISTS idx_series_last_discovered_at",
    // SQLite's ALTER TABLE DROP COLUMN landed in 3.35 but our minimum bar is
    // older; the column drop is a no-op, matching the repo's convention.
];
