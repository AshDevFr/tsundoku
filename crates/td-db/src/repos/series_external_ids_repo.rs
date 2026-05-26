//! Mapping table between internal `series.id` and per-provider external IDs.
//!
//! `UNIQUE(provider, external_id)` and `UNIQUE(series_id, provider)` are both
//! enforced in the schema. Callers that upsert a foreign ID inherited from a
//! provider's payload should tolerate conflict gracefully: another series
//! already claiming that `(provider, external_id)` is a data conflict the
//! review UI will surface, not a programmer error here.

use anyhow::Result;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use crate::entities::series_external_ids;

pub use series_external_ids::Model;

pub async fn upsert(
    db: &DatabaseConnection,
    series_id: i32,
    provider: &str,
    external_id: &str,
    external_url: Option<&str>,
    fetched_at: i64,
) -> Result<()> {
    let model = series_external_ids::ActiveModel {
        provider: Set(provider.to_string()),
        external_id: Set(external_id.to_string()),
        series_id: Set(series_id),
        external_url: Set(external_url.map(str::to_string)),
        fetched_at: Set(fetched_at),
    };
    series_external_ids::Entity::insert(model)
        .on_conflict(
            OnConflict::columns([
                series_external_ids::Column::Provider,
                series_external_ids::Column::ExternalId,
            ])
            .update_columns([
                series_external_ids::Column::SeriesId,
                series_external_ids::Column::ExternalUrl,
                series_external_ids::Column::FetchedAt,
            ])
            .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// Look up a series by a given provider's external ID. Returns `None` if the
/// mapping does not exist (the release resolver's first step).
pub async fn find_series_id(
    db: &DatabaseConnection,
    provider: &str,
    external_id: &str,
) -> Result<Option<i32>> {
    let row =
        series_external_ids::Entity::find_by_id((provider.to_string(), external_id.to_string()))
            .one(db)
            .await?;
    Ok(row.map(|r| r.series_id))
}

/// All external IDs known for a given internal series.
pub async fn list_for_series(db: &DatabaseConnection, series_id: i32) -> Result<Vec<Model>> {
    Ok(series_external_ids::Entity::find()
        .filter(series_external_ids::Column::SeriesId.eq(series_id))
        .all(db)
        .await?)
}

pub use series_external_ids::{ActiveModel, Column, Entity};
