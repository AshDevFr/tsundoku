pub use sea_orm_migration::prelude::*;

mod m20260524_000001_init;
mod m20260525_000001_genres_tags;
mod m20260525_000002_run_metrics;
mod m20260525_000003_observability;
mod m20260526_000001_mangaupdates_id_map;
mod m20260526_000002_release_search_queries;
mod m20260526_000003_series_description;
mod m20260526_000004_drop_series_genres_json;
mod m20260527_000001_series_volume_chapter_counts;
mod m20260527_000002_series_rating;
mod m20260527_000003_series_refresh_runs;

pub struct Migrator;

impl MigratorTrait for Migrator {
    /// Top-level schema followed by each provider's own nested migrations.
    /// Adding a new metadata provider with on-disk cache tables means
    /// importing its `migration::migrations()` here.
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        let mut m: Vec<Box<dyn MigrationTrait>> = vec![
            Box::new(m20260524_000001_init::Migration),
            Box::new(m20260525_000001_genres_tags::Migration),
            Box::new(m20260525_000002_run_metrics::Migration),
            Box::new(m20260525_000003_observability::Migration),
            Box::new(m20260526_000001_mangaupdates_id_map::Migration),
            Box::new(m20260526_000002_release_search_queries::Migration),
            Box::new(m20260526_000003_series_description::Migration),
            Box::new(m20260526_000004_drop_series_genres_json::Migration),
            Box::new(m20260527_000001_series_volume_chapter_counts::Migration),
            Box::new(m20260527_000002_series_rating::Migration),
            Box::new(m20260527_000003_series_refresh_runs::Migration),
        ];
        m.extend(td_metadata_mangabaka::migration::migrations());
        m
    }
}
