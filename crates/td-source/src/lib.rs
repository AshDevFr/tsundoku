//! Discovery-source abstraction.
//!
//! A [`DiscoverySource`] knows how to poll one upstream (Nyaa, future others)
//! and emit a vector of source-agnostic [`DiscoveredRelease`] DTOs. The
//! resolution pipeline, persistence layer, and scheduler never see a
//! provider-specific type: they consume `Vec<DiscoveredRelease>` and route
//! by `source_kind` only when they truly must (e.g. logging).
//!
//! The trait surface is small on purpose: anything source-specific (auth
//! shape, pagination, RSS vs JSON, ETag conventions) is an implementation
//! detail behind `poll()`. The caveat from the PRD: this shape is informed
//! by Nyaa only; expect a small refactor when the second source lands.
//!
//! Implementations live in separate crates (e.g. `td-source-nyaa`).

pub mod error;
pub mod format;
pub mod registry;
pub mod release;
pub mod source;
pub mod span;

pub use error::{SourceError, SourceResult};
pub use format::{Format, detect_formats};
pub use registry::{SourceRegistry, SourceRegistryBuilder};
pub use release::{DiscoveredRelease, ExternalLinks, PollContext, PollOutcome};
pub use source::{Backfillable, DiscoverySource};
pub use span::{
    ReleaseSpans, Span, detect_spans, merge_spans, spans_from_json, spans_max_end, spans_to_json,
};
