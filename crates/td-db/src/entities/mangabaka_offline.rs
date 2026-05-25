use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "mangabaka_offline")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub fetched_at: i64,
    pub dump_version: Option<String>,
    pub record_count: Option<i64>,
    pub source_url: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
