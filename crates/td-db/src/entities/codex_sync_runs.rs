use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Append-only per-sweep history for the Codex presence sync. One row per sweep
/// attempt (cron or manual); [`super::codex_status`] keeps only the latest
/// snapshot. `outcome` is `success` | `preflight_failed` | `auth_failed` |
/// `error`; `fetched_count` / `linked_count` are set only on `success`, `error`
/// only otherwise. `trigger` is `cron` | `manual`.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "codex_sync_runs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub ran_at: i64,
    #[sea_orm(column_name = "trigger")]
    pub trigger: String,
    pub outcome: String,
    pub fetched_count: Option<i64>,
    pub linked_count: Option<i64>,
    pub error: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
