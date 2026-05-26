pub use sea_orm_migration::prelude::*;

mod m20260524_000001_init;

pub struct Migrator;

impl MigratorTrait for Migrator {
    /// Top-level schema followed by each provider's own nested migrations.
    /// Adding a new metadata provider with on-disk cache tables means
    /// importing its `migration::migrations()` here.
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        let mut m: Vec<Box<dyn MigrationTrait>> = vec![Box::new(m20260524_000001_init::Migration)];
        m.extend(td_metadata_mangabaka::migration::migrations());
        m
    }
}
