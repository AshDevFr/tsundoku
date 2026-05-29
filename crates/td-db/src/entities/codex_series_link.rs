use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// One row per tsundoku series that maps to a Codex series. `link_kind` is
/// `"auto"` (matched by a shared external id during a sweep) or `"manual"`
/// (hand-linked by the operator). `local_max_*` are Codex's highest owned
/// volume/chapter and drive the presence status; `volumes_owned` is a soft,
/// display-only file count.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "codex_series_link")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub series_id: i32,
    pub codex_series_uuid: String,
    pub local_max_volume: Option<f64>,
    pub local_max_chapter: Option<f64>,
    pub volumes_owned: Option<i64>,
    pub link_kind: String,
    pub matched_provider: Option<String>,
    pub matched_external_id: Option<String>,
    pub synced_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::series::Entity",
        from = "Column::SeriesId",
        to = "super::series::Column::Id",
        on_delete = "Cascade"
    )]
    Series,
}

impl Related<super::series::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Series.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
