use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "release_formats")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub release_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub format: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::releases::Entity",
        from = "Column::ReleaseId",
        to = "super::releases::Column::Id",
        on_delete = "Cascade"
    )]
    Release,
}

impl Related<super::releases::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Release.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
