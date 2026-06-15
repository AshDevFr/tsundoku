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
mod m20260528_000001_run_progress;
mod m20260528_000002_poll_run_phase_timings;
mod m20260528_000003_release_information_url;
mod m20260529_000001_codex_presence;
mod m20260529_000002_codex_status_fetched_count;
mod m20260530_000001_release_comment_suggested_links;
mod m20260603_000001_release_sent_to_client;
mod m20260603_000001_series_ignore_completion;
mod m20260603_000002_download_status_history;
mod m20260603_000003_codex_health_checks;
mod m20260603_000004_codex_sync_runs;
mod m20260608_000001_series_coverage_and_updated_at;
mod m20260613_000001_series_wishlisted;
mod m20260614_000001_series_published_dates;

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
            Box::new(m20260528_000001_run_progress::Migration),
            Box::new(m20260528_000002_poll_run_phase_timings::Migration),
            Box::new(m20260528_000003_release_information_url::Migration),
            Box::new(m20260529_000001_codex_presence::Migration),
            Box::new(m20260529_000002_codex_status_fetched_count::Migration),
            Box::new(m20260530_000001_release_comment_suggested_links::Migration),
            Box::new(m20260603_000001_release_sent_to_client::Migration),
            Box::new(m20260603_000001_series_ignore_completion::Migration),
            Box::new(m20260603_000002_download_status_history::Migration),
            Box::new(m20260603_000003_codex_health_checks::Migration),
            Box::new(m20260603_000004_codex_sync_runs::Migration),
            Box::new(m20260608_000001_series_coverage_and_updated_at::Migration),
            Box::new(m20260613_000001_series_wishlisted::Migration),
            Box::new(m20260614_000001_series_published_dates::Migration),
        ];
        m.extend(td_metadata_mangabaka::migration::migrations());
        m
    }
}
