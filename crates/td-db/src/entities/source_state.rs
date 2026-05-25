use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "source_state")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub source_kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub source_name: String,
    pub etag: Option<String>,
    pub cursor: Option<String>,
    pub last_polled_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_error: Option<String>,
    pub last_summary: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
