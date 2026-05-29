use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Single-row (`id = 1`) Codex connection-health record. Surfaced by the admin
/// UI so reachability / version / auth state / last error are visible instead
/// of living only in the logs. `auth_state` is one of `unknown` | `ok` |
/// `unauthorized` (401) | `forbidden` (403).
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "codex_status")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i32,
    pub codex_name: Option<String>,
    pub codex_version: Option<String>,
    pub reachable: bool,
    pub auth_state: String,
    pub last_preflight_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_error: Option<String>,
    pub linked_count: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
