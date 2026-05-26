use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "provider_refreshes")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub provider_id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub bytes_downloaded: Option<i64>,
    pub record_count: Option<i64>,
    pub error_message: Option<String>,
    #[sea_orm(column_name = "trigger")]
    pub trigger: String,
    /// Wall-clock time spent inside `MetadataProvider::refresh_cache()` —
    /// most importantly the dump download — in milliseconds.
    pub fetch_duration_ms: Option<i64>,
    /// Coarse classification of the failure cause; see
    /// `td_scheduler::error_kind`.
    pub error_kind: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
