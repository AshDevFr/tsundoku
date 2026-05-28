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
    pub description: Option<String>,
    pub metadata_json: Option<String>,
    pub metadata_source: String,
    pub metadata_hash: Option<String>,
    pub metadata_fetched_at: i64,
    pub first_seen_at: i64,
    pub last_release_at: i64,
    pub highest_volume: Option<f64>,
    pub highest_chapter: Option<f64>,
    /// Published total volume/chapter counts from provider metadata
    /// (denormalized for display). Distinct from `highest_volume` /
    /// `highest_chapter`, which track the highest span seen across releases.
    pub total_volumes: Option<i32>,
    pub total_chapters: Option<i32>,
    pub owned: i32,
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
