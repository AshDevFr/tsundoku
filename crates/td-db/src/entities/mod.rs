//! sea-orm entities for the tsundoku schema.

pub mod provider_cache_state;
pub mod release_formats;
pub mod releases;
pub mod review_candidates;
pub mod series;
pub mod series_external_ids;
pub mod source_state;

pub use provider_cache_state::Entity as ProviderCacheState;
pub use release_formats::Entity as ReleaseFormats;
pub use releases::Entity as Releases;
pub use review_candidates::Entity as ReviewCandidates;
pub use series::Entity as Series;
pub use series_external_ids::Entity as SeriesExternalIds;
pub use source_state::Entity as SourceState;
