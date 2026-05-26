//! Provider-owned migrations.
//!
//! Composed into the top-level `Migrator` by `migration::lib::Migrator`.
//! Currently empty: the offline-cache schema (`mangabaka_offline_series`,
//! `mangabaka_offline_fts`) will land here once the offline dump format is
//! confirmed. Until then this module exists so the wiring contract
//! (`td_metadata_mangabaka::migration::migrations()`) is stable.

use sea_orm_migration::prelude::*;

/// Returns this provider's migrations in the order they should be applied.
/// Empty until offline cache tables land.
pub fn migrations() -> Vec<Box<dyn MigrationTrait>> {
    Vec::new()
}
