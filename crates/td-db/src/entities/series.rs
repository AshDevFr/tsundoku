use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "series")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub mangabaka_id: i32,
    pub title: String,
    pub alternate_titles_json: Option<String>,
    pub cover_url: Option<String>,
    #[sea_orm(column_name = "type")]
    pub kind: Option<String>,
    pub status: Option<String>,
    pub year: Option<i32>,
    pub genres_json: Option<String>,
    pub metadata_json: Option<String>,
    pub metadata_source: String,
    pub metadata_fetched_at: i64,
    pub first_seen_at: i64,
    pub last_release_at: i64,
    pub highest_volume: Option<f64>,
    pub highest_chapter: Option<f64>,
    pub owned: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::releases::Entity")]
    Releases,
}

impl Related<super::releases::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Releases.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
