use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "provider_cache_state")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub provider: String,
    pub fetched_at: i64,
    pub cache_version: Option<String>,
    pub record_count: Option<i64>,
    pub source_url: Option<String>,
    pub bytes_downloaded: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
