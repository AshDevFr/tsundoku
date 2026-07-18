use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Append-only audit for per-series release searches, one row per run
/// attempt (button or CLI). Inserted as `outcome = 'running'` when the walk
/// starts and completed in place (`success` | `error`) with its counts, so
/// the series page can poll the newest row for liveness. `trigger` is
/// `manual` | `cli`. `search_name` is the `[[search]]` entry name
/// (config-only, informational, not a FK).
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "search_runs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub ran_at: i64,
    pub finished_at: Option<i64>,
    pub search_name: String,
    pub series_id: i32,
    #[sea_orm(column_name = "trigger")]
    pub trigger: String,
    pub outcome: String,
    pub queries_attempted: Option<i64>,
    pub pages_fetched: Option<i64>,
    pub releases_seen: Option<i64>,
    pub releases_new: Option<i64>,
    pub error: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
