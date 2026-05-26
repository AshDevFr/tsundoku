use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "series_external_ids")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub provider: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub external_id: String,
    pub series_id: i32,
    pub external_url: Option<String>,
    pub fetched_at: i64,
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
