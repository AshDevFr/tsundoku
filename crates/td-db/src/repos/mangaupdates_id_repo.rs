//! Cache of MangaUpdates legacy-id → modern-id translations.
//!
//! MU migrated from numeric `series.html?id=NNN` IDs to base36
//! alphanumeric slugs in 2022. Uploader-pasted URLs still use the legacy
//! shape; MangaBaka's offline dump only knows the modern shape. We resolve
//! the gap by following MU's permanent-redirect from the legacy URL once
//! per id and persisting the result.
//!
//! A `modern_id` of `None` is a tombstone: MU redirected us somewhere that
//! does not look like a real series page (typically a bare `/series`),
//! meaning the legacy id has been retired. Tombstoned ids are dropped
//! from the resolver's candidate list rather than re-attempted on every
//! poll.

use anyhow::Result;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter,
    QuerySelect, Set,
};

use crate::entities::mangaupdates_id_map;

pub use mangaupdates_id_map::Model;

/// Aggregate counters over the persisted legacy → modern mapping table.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub modern_count: i64,
    pub tombstone_count: i64,
    pub last_resolved_at: Option<i64>,
}

#[derive(Debug, FromQueryResult)]
struct MaxResolvedAt {
    max_resolved_at: Option<i64>,
}

/// Outcome of a previous translation attempt, as far as the cache knows.
///
/// - `None` — the cache has never seen this legacy id; caller should
///   attempt the redirect and `record` the result.
/// - `Some(Some(modern))` — a valid mapping; caller can swap it in.
/// - `Some(None)` — tombstone; caller should drop the link.
pub type Lookup = Option<Option<String>>;

/// Read the cache for a single legacy id.
pub async fn lookup(db: &DatabaseConnection, legacy_id: i64) -> Result<Lookup> {
    let row = mangaupdates_id_map::Entity::find_by_id(legacy_id)
        .one(db)
        .await?;
    Ok(row.map(|r| r.modern_id))
}

/// Insert or overwrite the cache entry for `legacy_id`. Passing
/// `modern_id = None` tombstones the entry; subsequent `lookup` calls
/// return `Some(None)` and skip the network entirely.
pub async fn record(
    db: &DatabaseConnection,
    legacy_id: i64,
    modern_id: Option<&str>,
    resolved_at: i64,
) -> Result<()> {
    let row = mangaupdates_id_map::ActiveModel {
        legacy_id: Set(legacy_id),
        modern_id: Set(modern_id.map(str::to_string)),
        resolved_at: Set(resolved_at),
    };
    mangaupdates_id_map::Entity::insert(row)
        .on_conflict(
            OnConflict::column(mangaupdates_id_map::Column::LegacyId)
                .update_columns([
                    mangaupdates_id_map::Column::ModernId,
                    mangaupdates_id_map::Column::ResolvedAt,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// Aggregate counters for the admin "id maps" view: how many legacy ids
/// have a modern slug recorded, how many are tombstoned, and when the
/// most recent resolution happened.
pub async fn stats(db: &DatabaseConnection) -> Result<CacheStats> {
    let modern_count = mangaupdates_id_map::Entity::find()
        .filter(mangaupdates_id_map::Column::ModernId.is_not_null())
        .count(db)
        .await? as i64;
    let tombstone_count = mangaupdates_id_map::Entity::find()
        .filter(mangaupdates_id_map::Column::ModernId.is_null())
        .count(db)
        .await? as i64;
    let last = mangaupdates_id_map::Entity::find()
        .select_only()
        .column_as(
            mangaupdates_id_map::Column::ResolvedAt.max(),
            "max_resolved_at",
        )
        .into_model::<MaxResolvedAt>()
        .one(db)
        .await?
        .and_then(|r| r.max_resolved_at);
    Ok(CacheStats {
        modern_count,
        tombstone_count,
        last_resolved_at: last,
    })
}

pub use mangaupdates_id_map::{ActiveModel, Column, Entity};

#[cfg(test)]
mod tests {
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::Database;

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    #[tokio::test]
    async fn stats_on_empty_table_is_zero() {
        let db = fresh_db().await;
        let s = stats(&db).await.unwrap();
        assert_eq!(s.modern_count, 0);
        assert_eq!(s.tombstone_count, 0);
        assert_eq!(s.last_resolved_at, None);
    }

    #[tokio::test]
    async fn stats_counts_modern_and_tombstones_separately() {
        let db = fresh_db().await;
        record(&db, 1, Some("modern-a"), 100).await.unwrap();
        record(&db, 2, Some("modern-b"), 200).await.unwrap();
        record(&db, 3, None, 150).await.unwrap();
        let s = stats(&db).await.unwrap();
        assert_eq!(s.modern_count, 2);
        assert_eq!(s.tombstone_count, 1);
        assert_eq!(s.last_resolved_at, Some(200));
    }
}
