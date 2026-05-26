//! Mapping table between internal `series.id` and per-provider external IDs.
//!
//! `UNIQUE(provider, external_id)` and `UNIQUE(series_id, provider)` are both
//! enforced in the schema. Callers that upsert a foreign ID inherited from a
//! provider's payload should tolerate conflict gracefully: another series
//! already claiming that `(provider, external_id)` is a data conflict the
//! review UI will surface, not a programmer error here.

use anyhow::Result;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Set,
};

use crate::entities::series_external_ids;

pub use series_external_ids::Model;

/// `(provider, count)` pair returned by [`count_by_provider`].
#[derive(Debug, Clone, FromQueryResult)]
pub struct ProviderCount {
    pub provider: String,
    pub count: i64,
}

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

/// Row counts grouped by `provider`. Used by the admin id-maps view to
/// show how many foreign-id mappings each provider contributes.
pub async fn count_by_provider(db: &DatabaseConnection) -> Result<Vec<ProviderCount>> {
    let rows = series_external_ids::Entity::find()
        .select_only()
        .column(series_external_ids::Column::Provider)
        .column_as(series_external_ids::Column::Provider.count(), "count")
        .group_by(series_external_ids::Column::Provider)
        .order_by_asc(series_external_ids::Column::Provider)
        .into_model::<ProviderCount>()
        .all(db)
        .await?;
    Ok(rows)
}

pub use series_external_ids::{ActiveModel, Column, Entity};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::series;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{ActiveValue::Set, Database, EntityTrait};

    async fn fresh_db() -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    async fn insert_series(db: &sea_orm::DatabaseConnection, title: &str) -> i32 {
        let model = series::ActiveModel {
            canonical_title: Set(title.into()),
            metadata_source: Set("test".into()),
            metadata_fetched_at: Set(0),
            first_seen_at: Set(0),
            last_release_at: Set(0),
            owned: Set(0),
            ..Default::default()
        };
        series::Entity::insert(model)
            .exec_with_returning(db)
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn count_by_provider_groups_and_orders_alphabetically() {
        let db = fresh_db().await;
        let s1 = insert_series(&db, "A").await;
        let s2 = insert_series(&db, "B").await;
        upsert(&db, s1, "mangaupdates", "mu-1", None, 0)
            .await
            .unwrap();
        upsert(&db, s2, "mangaupdates", "mu-2", None, 0)
            .await
            .unwrap();
        upsert(&db, s1, "mal", "mal-1", None, 0).await.unwrap();
        let rows = count_by_provider(&db).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].provider, "mal");
        assert_eq!(rows[0].count, 1);
        assert_eq!(rows[1].provider, "mangaupdates");
        assert_eq!(rows[1].count, 2);
    }

    #[tokio::test]
    async fn count_by_provider_returns_empty_for_empty_table() {
        let db = fresh_db().await;
        let rows = count_by_provider(&db).await.unwrap();
        assert!(rows.is_empty());
    }
}
