//! Scheduled-job implementations.
//!
//! Each submodule exposes a single `build(cron, ...) -> Result<JobLocked>`
//! constructor that the scheduler crate composes during bootstrap. The job
//! closure owns clones of the shared state (`Arc`s and a `DatabaseConnection`,
//! both cheap to clone) and a handle to the per-key mutex map so overlapping
//! ticks are skipped rather than queued.

pub mod backfill_source;
pub mod outcomes;
pub mod poll_source;
pub mod progress;
pub mod reenrich_source;
pub mod refresh_provider_cache;
pub mod refresh_series_metadata;
pub mod search_series;
pub mod snapshot_review_queue;
pub mod sync_codex;
pub mod test_download;
