use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "series_genres")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub series_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    pub genre_id: i32,
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
    #[sea_orm(
        belongs_to = "super::genres::Entity",
        from = "Column::GenreId",
        to = "super::genres::Column::Id",
        on_delete = "Cascade"
    )]
    Genre,
}

impl ActiveModelBehavior for ActiveModel {}
