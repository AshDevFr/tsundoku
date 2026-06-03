use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Append-only download-client reachability history. A row is written only when
/// reachability *changes* or on a manual test, so a frequent health cron does
/// not flood the table; the current state lives in
/// [`super::download_status`]. `trigger` is one of `launch` | `cron` | `manual`.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "download_health_checks")]
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
