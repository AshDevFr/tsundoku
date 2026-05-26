//! The [`DiscoverySource`] trait.

use async_trait::async_trait;

use crate::error::SourceResult;
use crate::release::{PollContext, PollOutcome};

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
    async fn poll(&self, ctx: &PollContext) -> SourceResult<PollOutcome>;
}
