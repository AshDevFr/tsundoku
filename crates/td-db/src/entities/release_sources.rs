use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Which configured feeds carry a given release.
///
/// A release row is deduped on `(source_kind, external_id)` — one upstream
/// post is one row — but the same post routinely appears in several feeds, so
/// "who carries this" is many-to-many and lives here rather than in
/// `releases.source_name` (which records only the first discoverer).
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "release_sources")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub release_id: String,
    pub source_kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub source_name: String,
    /// When *this feed* first surfaced the release. Distinct from
    /// `releases.observed_at`, which is when any feed first did.
    pub first_seen_at: i64,
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
