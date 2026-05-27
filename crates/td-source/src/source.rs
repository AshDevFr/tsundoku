//! The [`DiscoverySource`] trait.

use async_trait::async_trait;

use crate::error::SourceResult;
use crate::release::{DiscoveredRelease, PollContext, PollOutcome};

#[async_trait]
pub trait DiscoverySource: Send + Sync {
    /// Config-defined instance name. Stable across restarts; doubles as the
    /// `source_name` column on persisted releases.
    fn name(&self) -> &str;

    /// Source kind (e.g. `"nyaa"`). Persisted as `source_kind`. Two sources
    /// of the same kind with different names are independent instances.
    fn kind(&self) -> &str;

    /// Poll this source and return new releases since the last successful
    /// poll. The caller hands in a [`PollContext`] describing the last-run
    /// state (ETag, cursor); the returned [`PollOutcome`] carries the new
    /// state for next time.
    ///
    /// Implementations should be idempotent: returning the same release
    /// twice is fine, persistence is keyed on `(source_kind, external_id)`.
    ///
    /// `poll` should be *fast*: parse-only, no per-release fan-out. Any
    /// per-release detail fetching belongs in [`Self::enrich`] so the
    /// scheduler can persist + resolve each release as soon as it's been
    /// enriched, instead of buffering an entire batch in memory and only
    /// surfacing it after the slowest item completes.
    async fn poll(&self, ctx: &PollContext) -> SourceResult<PollOutcome>;

    /// Optional per-release enrichment hook. Called by the scheduler
    /// immediately before persisting each release returned by
    /// [`Self::poll`]. Use it to fan out to detail pages, follow secondary
    /// URLs, or otherwise fill in fields that the bulk poll deliberately
    /// skipped. Default is a no-op for sources that don't need it.
    ///
    /// Failures here are non-fatal: the scheduler logs and persists the
    /// release with whatever data `poll` provided, so a flaky detail-page
    /// host can't sink an otherwise-good poll.
    async fn enrich(&self, _release: &mut DiscoveredRelease) -> SourceResult<()> {
        Ok(())
    }
}
