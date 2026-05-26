//! Resolution pipeline: maps raw `releases` rows to `series` rows.
//!
//! Walks a deterministic priority chain through the metadata registry:
//!
//! 1. **Known external ID** — look up each `(provider, foreign_id)` pair in
//!    `series_external_ids`. A hit short-circuits to the existing series row.
//! 2. **Foreign-ID lookup** — ask the active provider's
//!    [`MetadataProvider::resolve_by_foreign_id`] for each known link. The
//!    first hit drives a series upsert plus a fan-out of every cross-provider
//!    ID the active provider knows about.
//! 3. **Fuzzy title** — `active.search(title, N)` and re-score with the
//!    Dice coefficient against canonical + alternate titles. Above the
//!    `resolution_threshold` → resolved; below but plausible → review queue.
//! 4. **Format-type validation** — once a series is matched, check that the
//!    release's detected formats are consistent with the series's kind. A
//!    mismatch demotes the release to `ambiguous` for human review.
//!
//! The orchestrator is provider-agnostic by design: it only talks to
//! [`td_metadata::MetadataRegistry`]. Swapping the active provider in tests
//! (or in production) requires no code change here.
//!
//! [`MetadataProvider::resolve_by_foreign_id`]:
//!     td_metadata::MetadataProvider::resolve_by_foreign_id

pub mod foreign_id;
pub mod persist;
pub mod pipeline;
pub mod scoring;
pub mod validation;

pub use pipeline::{ResolutionOutcome, ResolutionPath, Resolver};
