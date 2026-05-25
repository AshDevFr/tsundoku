//! Source-state read/write helpers.

use anyhow::Result;
use sea_orm::sea_query::OnConflict;
use sea_orm::{DatabaseConnection, EntityTrait};

use crate::entities::source_state;

pub use source_state::Model;

pub async fn get(
    db: &DatabaseConnection,
    source_kind: &str,
    source_name: &str,
) -> Result<Option<Model>> {
    Ok(
        source_state::Entity::find_by_id((source_kind.to_string(), source_name.to_string()))
            .one(db)
            .await?,
    )
}

pub async fn upsert(db: &DatabaseConnection, model: source_state::ActiveModel) -> Result<()> {
    source_state::Entity::insert(model)
        .on_conflict(
            OnConflict::columns([
                source_state::Column::SourceKind,
                source_state::Column::SourceName,
            ])
            .update_columns([
                source_state::Column::Etag,
                source_state::Column::Cursor,
                source_state::Column::LastPolledAt,
                source_state::Column::LastSuccessAt,
                source_state::Column::LastError,
                source_state::Column::LastSummary,
            ])
            .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

pub use source_state::{ActiveModel, Column, Entity};
