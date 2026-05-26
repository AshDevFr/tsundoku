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

// Two new nullable columns on `releases` carrying JSON-array text:
//
// - `search_queries`: the queries the title cleaner produced for this
//   release (longest-first; usually one entry, more when the raw title
//   had a romaji / English split on `|` or ` / `).
// - `cleanup_rules_applied`: stable rule names that fired during cleanup
//   (e.g. `strip_parens`, `strip_vol_compact`, `split_alternates`). The
//   review UI surfaces these as badge chips so the operator can see what
//   surgery happened.
//
// Both columns are nullable: existing rows stay NULL until the next
// resolve cycle backfills them.
const UP: &[&str] = &[
    "ALTER TABLE releases ADD COLUMN search_queries TEXT",
    "ALTER TABLE releases ADD COLUMN cleanup_rules_applied TEXT",
];

const DOWN: &[&str] = &[
    // SQLite's ALTER TABLE DROP COLUMN landed in 3.35 but our minimum
    // bar is older; relying on table-rebuild is overkill for two
    // diagnostic columns. Partial down is a no-op.
];
