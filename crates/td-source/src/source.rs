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

    /// Downcast to [`Backfillable`] if this source supports historical
    /// backfill (walking older listing pages, replaying a cursor, etc.).
    /// Returns `None` for sources that only expose a steady-state poll.
    ///
    /// The `backfill` CLI surface uses this to dispatch without string-
    /// matching on `kind()`. Default is `None`; opt in by overriding.
    fn as_backfillable(&self) -> Option<&dyn Backfillable> {
        None
    }
}

/// Opt-in companion to [`DiscoverySource`]: a source that can replay
/// historical releases page by page. The CLI's `backfill` command drives
/// this in a loop, persisting and resolving between pages so a mid-run
/// abort still preserves the work done so far.
///
/// The "page" abstraction is intentionally coarse: each implementer maps
/// `page` onto whatever the upstream supports (HTML listing pagination,
/// date-bucketed cursors, offset windows). Returning an empty Vec is the
/// signal that the source has no more history to surface; the CLI stops
/// walking forward.
///
/// Backfill output goes through the same per-release pipeline as poll:
/// the caller is expected to run [`DiscoverySource::enrich`] +
/// `persist_discovered` + the resolver on each returned release.
#[async_trait]
pub trait Backfillable: Send + Sync {
    /// Fetch one page of historical releases. `page` is 1-indexed.
    /// Returning an empty Vec means "no more pages"; the caller stops.
    async fn backfill_page(&self, page: u32) -> SourceResult<Vec<DiscoveredRelease>>;
}
