//! sea-orm entities for the tsundoku schema.

pub mod mangabaka_offline;
pub mod release_formats;
pub mod releases;
pub mod review_candidates;
pub mod series;
pub mod source_state;

pub use mangabaka_offline::Entity as MangabakaOffline;
pub use release_formats::Entity as ReleaseFormats;
pub use releases::Entity as Releases;
pub use review_candidates::Entity as ReviewCandidates;
pub use series::Entity as Series;
pub use source_state::Entity as SourceState;
