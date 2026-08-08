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
    fetched_at: i64,
) -> Result<()> {
    let model = series_external_ids::ActiveModel {
        provider: Set(provider.to_string()),
        external_id: Set(external_id.to_string()),
        series_id: Set(series_id),
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

/// Every mapping carrying this `external_id`, across all providers, ordered
/// by provider for a stable presentation.
///
/// Distinct from [`find_series_id`], which is provider-qualified and therefore
/// 0-or-1 by the `UNIQUE(provider, external_id)` constraint. A *bare* id has
/// no such guarantee: provider id spaces overlap freely, so the same number is
/// a different series on MAL than on MangaBaka. Measured on a real catalog,
/// ~3% of distinct external ids map to more than one series, up to four. The
/// caller is expected to disambiguate rather than silently pick one.
pub async fn find_by_external_id(db: &DatabaseConnection, external_id: &str) -> Result<Vec<Model>> {
    Ok(series_external_ids::Entity::find()
        .filter(series_external_ids::Column::ExternalId.eq(external_id))
        .order_by_asc(series_external_ids::Column::Provider)
        .all(db)
        .await?)
}

/// All external IDs known for a given internal series.
pub async fn list_for_series(db: &DatabaseConnection, series_id: i32) -> Result<Vec<Model>> {
    Ok(series_external_ids::Entity::find()
        .filter(series_external_ids::Column::SeriesId.eq(series_id))
        .all(db)
        .await?)
}

/// All external IDs for a batch of series, grouped by `series_id`. Series
/// with no mappings are omitted (callers treat absence as an empty list).
/// One SELECT used by the catalog export to avoid an N+1 over the page.
pub async fn by_series_ids(
    db: &DatabaseConnection,
    series_ids: &[i32],
) -> Result<std::collections::HashMap<i32, Vec<Model>>> {
    if series_ids.is_empty() {
        return Ok(Default::default());
    }
    let rows = series_external_ids::Entity::find()
        .filter(series_external_ids::Column::SeriesId.is_in(series_ids.iter().copied()))
        .all(db)
        .await?;
    let mut map: std::collections::HashMap<i32, Vec<Model>> = std::collections::HashMap::new();
    for row in rows {
        map.entry(row.series_id).or_default().push(row);
    }
    Ok(map)
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
        upsert(&db, s1, "mangaupdates", "mu-1", 0).await.unwrap();
        upsert(&db, s2, "mangaupdates", "mu-2", 0).await.unwrap();
        upsert(&db, s1, "mal", "mal-1", 0).await.unwrap();
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

    #[tokio::test]
    async fn by_series_ids_groups_per_series_and_omits_empties() {
        let db = fresh_db().await;
        let s1 = insert_series(&db, "A").await;
        let s2 = insert_series(&db, "B").await;
        let s3 = insert_series(&db, "C").await;
        upsert(&db, s1, "mal", "mal-1", 0).await.unwrap();
        upsert(&db, s1, "anilist", "al-1", 0).await.unwrap();
        upsert(&db, s2, "mal", "mal-2", 0).await.unwrap();
        // s3 has no mappings.

        let map = by_series_ids(&db, &[s1, s2, s3]).await.unwrap();
        assert_eq!(map.len(), 2, "series with no mappings are omitted");
        assert_eq!(map[&s1].len(), 2);
        assert_eq!(map[&s2].len(), 1);
        assert!(!map.contains_key(&s3));
    }

    /// A bare id is ambiguous across providers by construction — the same
    /// number is a different series on each — so the lookup returns the whole
    /// set for the caller to disambiguate.
    #[tokio::test]
    async fn find_by_external_id_spans_providers_and_orders_by_provider() {
        let db = fresh_db().await;
        let s1 = insert_series(&db, "A").await;
        let s2 = insert_series(&db, "B").await;
        let s3 = insert_series(&db, "C").await;
        upsert(&db, s1, "mangabaka", "1329", 0).await.unwrap();
        upsert(&db, s2, "mal", "1329", 0).await.unwrap();
        upsert(&db, s3, "kitsu", "9999", 0).await.unwrap();

        let hits = find_by_external_id(&db, "1329").await.unwrap();
        assert_eq!(
            hits.iter()
                .map(|m| (m.provider.as_str(), m.series_id))
                .collect::<Vec<_>>(),
            vec![("mal", s2), ("mangabaka", s1)],
            "both providers' rows come back, ordered by provider",
        );

        assert_eq!(find_by_external_id(&db, "9999").await.unwrap().len(), 1);
        assert!(find_by_external_id(&db, "nope").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn by_series_ids_empty_input_is_empty_map() {
        let db = fresh_db().await;
        let map = by_series_ids(&db, &[]).await.unwrap();
        assert!(map.is_empty());
    }
}
