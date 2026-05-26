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

// Phase 3 extended observability: extra columns on the two run-history
// tables so the admin metrics view can render the resolution-outcome
// breakdown, error-kind donut, and fetch-latency percentiles. Plus a new
// `review_queue_snapshots` table for the depth-over-time chart and a
// `releases.resolved_at` column so the time-to-resolution histogram has a
// stable anchor.
//
// Schema deltas are kept ALTER-only here — the parent tables stay defined
// in the migration that created them.
const UP: &[&str] = &[
    // poll_runs: fetch_duration_ms (just the source.poll() call, not the
    // surrounding persist/resolve loop), error_kind, and one counter per
    // ResolutionOutcome variant per tick.
    "ALTER TABLE poll_runs ADD COLUMN fetch_duration_ms INTEGER",
    "ALTER TABLE poll_runs ADD COLUMN error_kind TEXT",
    "ALTER TABLE poll_runs ADD COLUMN outcome_known_id INTEGER",
    "ALTER TABLE poll_runs ADD COLUMN outcome_foreign_id INTEGER",
    "ALTER TABLE poll_runs ADD COLUMN outcome_fuzzy INTEGER",
    "ALTER TABLE poll_runs ADD COLUMN outcome_review INTEGER",
    "ALTER TABLE poll_runs ADD COLUMN outcome_failed INTEGER",
    // provider_refreshes: same fetch_duration_ms (the dump download) and
    // shared error_kind classification.
    "ALTER TABLE provider_refreshes ADD COLUMN fetch_duration_ms INTEGER",
    "ALTER TABLE provider_refreshes ADD COLUMN error_kind TEXT",
    // releases.resolved_at: set when link_release writes status='resolved'.
    // Lets the metrics layer compute time-to-resolution percentiles without
    // a separate event log.
    "ALTER TABLE releases ADD COLUMN resolved_at INTEGER",
    "CREATE INDEX ix_releases_resolved_at ON releases(resolved_at)",
    // Hourly review-queue depth snapshot. `oldest_pending_seconds` is the
    // age of the oldest unresolved release in the queue at snapshot time.
    "CREATE TABLE review_queue_snapshots (
        id                       INTEGER PRIMARY KEY AUTOINCREMENT,
        captured_at              INTEGER NOT NULL,
        pending_count            INTEGER NOT NULL,
        unresolved_count         INTEGER NOT NULL,
        ambiguous_count          INTEGER NOT NULL,
        review_pending_count     INTEGER NOT NULL,
        oldest_pending_seconds   INTEGER
    )",
    "CREATE INDEX ix_review_queue_snapshots_captured
        ON review_queue_snapshots(captured_at DESC)",
];

const DOWN: &[&str] = &[
    "DROP INDEX IF EXISTS ix_review_queue_snapshots_captured",
    "DROP TABLE IF EXISTS review_queue_snapshots",
    "DROP INDEX IF EXISTS ix_releases_resolved_at",
    // SQLite doesn't support DROP COLUMN in older versions; relying on
    // table-rebuild is heavier than this admin-side data warrants. A full
    // rollback drops the migration entirely; partial down is a no-op.
];
