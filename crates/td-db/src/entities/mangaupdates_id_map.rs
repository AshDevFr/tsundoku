use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "mangaupdates_id_map")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub legacy_id: i64,
    /// `Some(slug)` once MangaUpdates' redirect resolved to a real series
    /// page. `None` is a tombstone: the legacy id no longer maps to
    /// anything (MU redirected us to `/series` without a slug).
    pub modern_id: Option<String>,
    pub resolved_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
