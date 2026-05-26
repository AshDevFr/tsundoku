use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "review_queue_snapshots")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub captured_at: i64,
    pub pending_count: i64,
    pub unresolved_count: i64,
    pub ambiguous_count: i64,
    pub review_pending_count: i64,
    pub oldest_pending_seconds: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
