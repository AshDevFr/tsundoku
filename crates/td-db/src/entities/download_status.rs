use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Single-row (`id = 1`) download-client connection-health snapshot. Rewritten
/// on every probe (launch / cron / manual) so the admin UI can show
/// reachability and the last-test time. The append-only history lives in
/// [`super::download_health_checks`]; `last_change_at` records when reachability
/// last flipped.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "download_status")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i32,
    pub reachable: bool,
    pub last_test_at: Option<i64>,
    pub last_change_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
