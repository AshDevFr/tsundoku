use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "series")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub canonical_title: String,
    pub alternate_titles_json: Option<String>,
    pub cover_url: Option<String>,
    #[sea_orm(column_name = "type")]
    pub kind: Option<String>,
    pub status: Option<String>,
    pub year: Option<i32>,
    /// Official publication start/end dates from provider metadata, ISO
    /// `YYYY-MM-DD` strings (nullable). Stored as TEXT so `ORDER BY` sorts
    /// them lexicographically; the feed's "Publication date" sort uses
    /// `published_start_date`. Distinct from `last_release_at` (the last
    /// *discovered* release).
    pub published_start_date: Option<String>,
    pub published_end_date: Option<String>,
    pub description: Option<String>,
    /// Gap-preserving merged volume coverage across all linked releases: a JSON
    /// array of `{start,end}` (`td_source::Span`), or NULL when none parsed.
    /// Maintained by `releases_repo::recompute_series_coverage`; the max end
    /// equals `highest_volume`.
    pub volume_coverage_json: Option<String>,
    /// Gap-preserving merged chapter coverage; same shape as
    /// `volume_coverage_json`. Max end equals `highest_chapter`.
    pub chapter_coverage_json: Option<String>,
    /// Epoch seconds the series' release coverage (or `highest_*`) last changed.
    /// Bumped only on real coverage changes — never by a metadata refresh — so
    /// it drives the incremental release feed's `(updated_at, id)` cursor.
    pub updated_at: i64,
    pub metadata_json: Option<String>,
    pub metadata_source: String,
    pub metadata_hash: Option<String>,
    pub metadata_fetched_at: i64,
    pub first_seen_at: i64,
    pub last_release_at: i64,
    /// Epoch seconds tsundoku last *discovered* a release for this series —
    /// `MAX(observed_at)` across linked releases. Distinct from
    /// `last_release_at`, which is the newest linked release's upstream post
    /// date; the two diverge by months whenever a source surfaces an old post
    /// (query feeds, backfills, the per-series release search). NULL until
    /// something links. Maintained by
    /// `releases_repo::recompute_series_coverage` with the other
    /// release-derived aggregates, so it shrinks correctly when a release
    /// moves away.
    pub last_discovered_at: Option<i64>,
    pub highest_volume: Option<f64>,
    pub highest_chapter: Option<f64>,
    /// Published total volume/chapter counts from provider metadata
    /// (denormalized for display). Distinct from `highest_volume` /
    /// `highest_chapter`, which track the highest span seen across releases.
    pub total_volumes: Option<i32>,
    pub total_chapters: Option<i32>,
    /// Provider rating on the canonical 0-10 scale (normalized by the
    /// provider's mapping layer). Denormalized for display alongside the
    /// counts; not used by the resolver.
    pub rating: Option<f64>,
    pub owned: i32,
    /// Admin-only wishlist flag, stored as the epoch-second timestamp the
    /// series was clipped (NULL ⇒ not wishlisted). `IS NOT NULL` is the flag;
    /// the value drives the wishlist view's "recently clipped" sort. Operator-
    /// owned: a metadata refresh never touches it, and it is independent of
    /// `owned` (removal is manual only).
    pub wishlisted_at: Option<i64>,
    /// Operator opt-out of Codex completion tracking. When set, the series'
    /// Codex status is forced to `Ignored` regardless of discovered vs owned
    /// maxima — used for series read in omnibus, where source (single-volume)
    /// numbering is permanently ahead of owned (omnibus) numbering and the
    /// "Behind" signal is structurally noise. Never written by metadata
    /// refresh (the refresh UPDATE leaves operator-owned columns `NotSet`).
    pub ignore_completion: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::releases::Entity")]
    Releases,
    #[sea_orm(has_many = "super::series_external_ids::Entity")]
    ExternalIds,
    #[sea_orm(has_many = "super::review_candidates::Entity")]
    ReviewCandidates,
}

impl Related<super::releases::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Releases.def()
    }
}

impl Related<super::series_external_ids::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ExternalIds.def()
    }
}

impl Related<super::review_candidates::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ReviewCandidates.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
