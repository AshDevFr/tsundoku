use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Append-only Codex reachability history, the same shape as
/// [`super::download_health_checks`]: a row is written only on a reachability
/// change or a manual test. The current snapshot lives in
/// [`super::codex_status`]. `trigger` is one of `launch` | `cron` | `manual`.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "codex_health_checks")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub checked_at: i64,
    pub reachable: bool,
    pub error: Option<String>,
    #[sea_orm(column_name = "trigger")]
    pub trigger: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
