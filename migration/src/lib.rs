pub use sea_orm_migration::prelude::*;

mod m20250101_000001_create_example;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20250101_000001_create_example::Migration)]
    }
}
