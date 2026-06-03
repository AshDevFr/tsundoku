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

// Operator opt-out of Codex completion tracking. When set, a series' Codex
// status is forced to `ignored` regardless of discovered-vs-owned maxima.
// Used for series read in omnibus, where source (single-volume) numbering is
// permanently ahead of owned (omnibus) numbering, so the "behind" signal is
// structurally noise. Default 0 = current behaviour (tracked); never written
// by metadata refresh (the refresh UPDATE leaves operator-owned columns alone).
const UP: &[&str] = &["ALTER TABLE series ADD COLUMN ignore_completion BOOLEAN NOT NULL DEFAULT 0"];

const DOWN: &[&str] = &[
    // SQLite's ALTER TABLE DROP COLUMN landed in 3.35 but our minimum
    // bar is older; a partial down is a no-op.
];
