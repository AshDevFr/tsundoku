//! Thin query helpers over the sea-orm entities.

/// `trigger` discriminants for the connection-health history tables, shared by
/// the download and codex status repos so the two stay in lock-step.
pub const TRIGGER_LAUNCH: &str = "launch";
pub const TRIGGER_CRON: &str = "cron";
pub const TRIGGER_MANUAL: &str = "manual";

pub mod codex_link_repo;
pub mod codex_status_repo;
pub mod codex_sync_runs_repo;
pub mod download_sends_repo;
pub mod download_status_repo;
pub mod mangaupdates_id_repo;
pub mod provider_cache_state_repo;
pub mod releases_repo;
pub mod review_repo;
pub mod review_snapshots_repo;
pub mod run_metrics_repo;
pub mod search_runs_repo;
pub mod series_external_ids_repo;
pub mod series_refresh_repo;
pub mod series_repo;
pub mod sources_repo;
pub mod tagging_repo;
