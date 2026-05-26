//! sea-orm entities for the tsundoku schema.

pub mod genres;
pub mod provider_cache_state;
pub mod release_formats;
pub mod releases;
pub mod review_candidates;
pub mod series;
pub mod series_external_ids;
pub mod series_genres;
pub mod series_tags;
pub mod source_state;
pub mod tags;

pub use genres::Entity as Genres;
pub use provider_cache_state::Entity as ProviderCacheState;
pub use release_formats::Entity as ReleaseFormats;
pub use releases::Entity as Releases;
pub use review_candidates::Entity as ReviewCandidates;
pub use series::Entity as Series;
pub use series_external_ids::Entity as SeriesExternalIds;
pub use series_genres::Entity as SeriesGenres;
pub use series_tags::Entity as SeriesTags;
pub use source_state::Entity as SourceState;
pub use tags::Entity as Tags;
