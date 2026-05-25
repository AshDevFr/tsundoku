pub use sea_orm_migration::prelude::*;

mod m20260524_000001_init;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20260524_000001_init::Migration)]
    }
}
