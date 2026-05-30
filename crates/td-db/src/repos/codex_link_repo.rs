//! Codex series-link mapping: which tsundoku series map to a Codex series.
//!
//! Two link kinds share the table. `auto` links are (re)written by the sync
//! sweep from a shared external id; `manual` links are created by the operator
//! for series with no matchable external id. The sweep must never clobber a
//! manual link, so [`upsert_auto`] only writes when the existing row (if any)
//! is itself `auto`, and [`delete_stale_auto`] only prunes `auto` rows.

use anyhow::Result;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, Set};

use crate::entities::codex_series_link;

pub use codex_series_link::{ActiveModel, Column, Entity, Model};

/// `link_kind` discriminants. Stored as plain text so a future kind doesn't
/// need a migration.
pub const KIND_AUTO: &str = "auto";
pub const KIND_MANUAL: &str = "manual";

/// Fields of an automatically-matched link, captured from one Codex
/// `external-index` item plus the local series it matched.
#[derive(Debug, Clone)]
pub struct AutoLink {
    pub series_id: i32,
    pub codex_series_uuid: String,
    pub local_max_volume: Option<f64>,
    pub local_max_chapter: Option<f64>,
    pub volumes_owned: Option<i64>,
    pub matched_provider: String,
    pub matched_external_id: String,
    pub synced_at: i64,
}

/// Insert or refresh an `auto` link. A pre-existing `manual` link for the same
/// series wins: the `ON CONFLICT ... WHERE link_kind = 'auto'` guard turns the
/// write into a no-op rather than demoting the manual link to auto.
pub async fn upsert_auto(db: &DatabaseConnection, link: &AutoLink) -> Result<()> {
    let model = ActiveModel {
        series_id: Set(link.series_id),
        codex_series_uuid: Set(link.codex_series_uuid.clone()),
        local_max_volume: Set(link.local_max_volume),
        local_max_chapter: Set(link.local_max_chapter),
        volumes_owned: Set(link.volumes_owned),
        link_kind: Set(KIND_AUTO.to_string()),
        matched_provider: Set(Some(link.matched_provider.clone())),
        matched_external_id: Set(Some(link.matched_external_id.clone())),
        synced_at: Set(link.synced_at),
    };
    let res = Entity::insert(model)
        .on_conflict(
            OnConflict::column(Column::SeriesId)
                .update_columns([
                    Column::CodexSeriesUuid,
                    Column::LocalMaxVolume,
                    Column::LocalMaxChapter,
                    Column::VolumesOwned,
                    Column::LinkKind,
                    Column::MatchedProvider,
                    Column::MatchedExternalId,
                    Column::SyncedAt,
                ])
                .action_and_where(Expr::col(Column::LinkKind).eq(KIND_AUTO))
                .to_owned(),
        )
        .exec(db)
        .await;
    // A conflict against a `manual` row leaves the WHERE false, so SQLite does
    // nothing and sea-orm reports `RecordNotInserted`. That is the intended
    // "manual wins" outcome, not an error.
    match res {
        Ok(_) | Err(sea_orm::DbErr::RecordNotInserted) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Insert or replace a `manual` link. Manual always wins, so this overwrites
/// whatever row exists (including a prior `auto` match). Counts are left to the
/// next sweep, which refreshes them by `codex_series_uuid`.
pub async fn upsert_manual(
    db: &DatabaseConnection,
    series_id: i32,
    codex_series_uuid: &str,
    synced_at: i64,
) -> Result<()> {
    let model = ActiveModel {
        series_id: Set(series_id),
        codex_series_uuid: Set(codex_series_uuid.to_string()),
        local_max_volume: Set(None),
        local_max_chapter: Set(None),
        volumes_owned: Set(None),
        link_kind: Set(KIND_MANUAL.to_string()),
        matched_provider: Set(None),
        matched_external_id: Set(None),
        synced_at: Set(synced_at),
    };
    Entity::insert(model)
        .on_conflict(
            OnConflict::column(Column::SeriesId)
                .update_columns([
                    Column::CodexSeriesUuid,
                    Column::LinkKind,
                    Column::LocalMaxVolume,
                    Column::LocalMaxChapter,
                    Column::VolumesOwned,
                    Column::MatchedProvider,
                    Column::MatchedExternalId,
                    Column::SyncedAt,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// Remove the link for a series regardless of kind (operator unlink).
pub async fn delete(db: &DatabaseConnection, series_id: i32) -> Result<()> {
    Entity::delete_by_id(series_id).exec(db).await?;
    Ok(())
}

/// Delete every `auto` link whose `series_id` is not in `alive_series_ids` —
/// the series whose Codex match disappeared since the last sweep. `manual`
/// links are never touched.
pub async fn delete_stale_auto(db: &DatabaseConnection, alive_series_ids: &[i32]) -> Result<u64> {
    let mut cond = Column::LinkKind.eq(KIND_AUTO);
    if !alive_series_ids.is_empty() {
        cond = cond.and(Column::SeriesId.is_not_in(alive_series_ids.iter().copied()));
    }
    let res = Entity::delete_many().filter(cond).exec(db).await?;
    Ok(res.rows_affected)
}

/// The link for one series, if any.
pub async fn get(db: &DatabaseConnection, series_id: i32) -> Result<Option<Model>> {
    Ok(Entity::find_by_id(series_id).one(db).await?)
}

/// Every link, both kinds. Used by the admin series filter to compute the
/// presence status of all linked series at once (personal scale, so loading
/// the whole table is cheap and keeps the status logic in one place).
pub async fn list_all(db: &DatabaseConnection) -> Result<Vec<Model>> {
    Ok(Entity::find().all(db).await?)
}

/// All `manual` links. The sweep refreshes their counts by matching
/// `codex_series_uuid` against the swept items (a manual link is created
/// without counts, since it has no external-id match to carry them).
pub async fn list_manual(db: &DatabaseConnection) -> Result<Vec<Model>> {
    Ok(Entity::find()
        .filter(Column::LinkKind.eq(KIND_MANUAL))
        .all(db)
        .await?)
}

/// Refresh just the Codex-sourced count columns for an existing link,
/// regardless of kind. Used to push fresh `local_max_*` / `volumes_owned`
/// onto a `manual` link whose `codex_series_uuid` matched a swept item. A
/// no-op if the series has no link row.
pub async fn update_counts(
    db: &DatabaseConnection,
    series_id: i32,
    local_max_volume: Option<f64>,
    local_max_chapter: Option<f64>,
    volumes_owned: Option<i64>,
    synced_at: i64,
) -> Result<()> {
    Entity::update_many()
        .col_expr(Column::LocalMaxVolume, Expr::value(local_max_volume))
        .col_expr(Column::LocalMaxChapter, Expr::value(local_max_chapter))
        .col_expr(Column::VolumesOwned, Expr::value(volumes_owned))
        .col_expr(Column::SyncedAt, Expr::value(synced_at))
        .filter(Column::SeriesId.eq(series_id))
        .exec(db)
        .await?;
    Ok(())
}

/// Links for a batch of series, for the list/detail join. Series with no link
/// are simply absent from the result.
pub async fn get_for_series_ids(db: &DatabaseConnection, series_ids: &[i32]) -> Result<Vec<Model>> {
    if series_ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(Entity::find()
        .filter(Column::SeriesId.is_in(series_ids.iter().copied()))
        .all(db)
        .await?)
}

/// Total number of links (auto + manual). Used for the status row's
/// `linked_count`.
pub async fn count(db: &DatabaseConnection) -> Result<i64> {
    Ok(Entity::find().count(db).await? as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::series;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{ActiveValue::Set as AvSet, Database};

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    async fn insert_series(db: &DatabaseConnection, title: &str) -> i32 {
        let model = series::ActiveModel {
            canonical_title: AvSet(title.into()),
            metadata_source: AvSet("test".into()),
            metadata_fetched_at: AvSet(0),
            first_seen_at: AvSet(0),
            last_release_at: AvSet(0),
            owned: AvSet(0),
            ..Default::default()
        };
        series::Entity::insert(model)
            .exec_with_returning(db)
            .await
            .unwrap()
            .id
    }

    fn auto(series_id: i32, uuid: &str, vol: Option<f64>, synced_at: i64) -> AutoLink {
        AutoLink {
            series_id,
            codex_series_uuid: uuid.into(),
            local_max_volume: vol,
            local_max_chapter: None,
            volumes_owned: vol.map(|v| v as i64),
            matched_provider: "mangabaka".into(),
            matched_external_id: "123".into(),
            synced_at,
        }
    }

    #[tokio::test]
    async fn upsert_auto_inserts_then_updates() {
        let db = fresh_db().await;
        let s = insert_series(&db, "A").await;

        upsert_auto(&db, &auto(s, "uuid-1", Some(3.0), 100))
            .await
            .unwrap();
        let row = get(&db, s).await.unwrap().unwrap();
        assert_eq!(row.codex_series_uuid, "uuid-1");
        assert_eq!(row.local_max_volume, Some(3.0));
        assert_eq!(row.link_kind, KIND_AUTO);

        // A second auto upsert refreshes the same row in place.
        upsert_auto(&db, &auto(s, "uuid-1", Some(7.0), 200))
            .await
            .unwrap();
        let row = get(&db, s).await.unwrap().unwrap();
        assert_eq!(row.local_max_volume, Some(7.0));
        assert_eq!(row.synced_at, 200);
        assert_eq!(count(&db).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn auto_upsert_does_not_clobber_manual_link() {
        let db = fresh_db().await;
        let s = insert_series(&db, "A").await;

        upsert_manual(&db, s, "manual-uuid", 50).await.unwrap();
        // Sweep tries to write an auto link for the same series.
        upsert_auto(&db, &auto(s, "auto-uuid", Some(9.0), 100))
            .await
            .unwrap();

        let row = get(&db, s).await.unwrap().unwrap();
        assert_eq!(row.link_kind, KIND_MANUAL, "manual link must survive");
        assert_eq!(row.codex_series_uuid, "manual-uuid");
        assert!(row.local_max_volume.is_none());
    }

    #[tokio::test]
    async fn delete_stale_auto_removes_missing_auto_keeps_manual() {
        let db = fresh_db().await;
        let s1 = insert_series(&db, "A").await;
        let s2 = insert_series(&db, "B").await;
        let s3 = insert_series(&db, "C").await;

        upsert_auto(&db, &auto(s1, "u1", Some(1.0), 100))
            .await
            .unwrap();
        upsert_auto(&db, &auto(s2, "u2", Some(2.0), 100))
            .await
            .unwrap();
        upsert_manual(&db, s3, "u3", 100).await.unwrap();

        // Only s1 is still matched this sweep. s2's auto link is stale; s3 is
        // manual and must be preserved even though it isn't in the alive set.
        let removed = delete_stale_auto(&db, &[s1]).await.unwrap();
        assert_eq!(removed, 1);
        assert!(get(&db, s1).await.unwrap().is_some());
        assert!(get(&db, s2).await.unwrap().is_none());
        assert!(get(&db, s3).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn delete_stale_auto_with_empty_alive_clears_all_auto() {
        let db = fresh_db().await;
        let s1 = insert_series(&db, "A").await;
        let s2 = insert_series(&db, "B").await;
        upsert_auto(&db, &auto(s1, "u1", None, 100)).await.unwrap();
        upsert_manual(&db, s2, "u2", 100).await.unwrap();

        let removed = delete_stale_auto(&db, &[]).await.unwrap();
        assert_eq!(removed, 1);
        assert!(get(&db, s1).await.unwrap().is_none());
        assert!(get(&db, s2).await.unwrap().is_some(), "manual preserved");
    }

    #[tokio::test]
    async fn get_for_series_ids_batches_and_skips_unlinked() {
        let db = fresh_db().await;
        let s1 = insert_series(&db, "A").await;
        let s2 = insert_series(&db, "B").await;
        let s3 = insert_series(&db, "C").await;
        upsert_auto(&db, &auto(s1, "u1", Some(1.0), 1))
            .await
            .unwrap();
        upsert_manual(&db, s3, "u3", 1).await.unwrap();

        let rows = get_for_series_ids(&db, &[s1, s2, s3]).await.unwrap();
        assert_eq!(rows.len(), 2, "s2 has no link, so it is absent");
        assert!(get_for_series_ids(&db, &[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn update_counts_refreshes_manual_link_in_place() {
        let db = fresh_db().await;
        let s = insert_series(&db, "A").await;
        upsert_manual(&db, s, "manual-uuid", 10).await.unwrap();

        // A manual link starts with no counts; the sweep pushes them by uuid.
        update_counts(&db, s, Some(8.0), Some(95.5), Some(8), 200)
            .await
            .unwrap();
        let row = get(&db, s).await.unwrap().unwrap();
        assert_eq!(row.link_kind, KIND_MANUAL, "kind unchanged");
        assert_eq!(row.codex_series_uuid, "manual-uuid", "uuid unchanged");
        assert_eq!(row.local_max_volume, Some(8.0));
        assert_eq!(row.local_max_chapter, Some(95.5));
        assert_eq!(row.volumes_owned, Some(8));
        assert_eq!(row.synced_at, 200);
    }

    #[tokio::test]
    async fn list_manual_returns_only_manual_links() {
        let db = fresh_db().await;
        let s1 = insert_series(&db, "A").await;
        let s2 = insert_series(&db, "B").await;
        upsert_auto(&db, &auto(s1, "u1", Some(1.0), 1))
            .await
            .unwrap();
        upsert_manual(&db, s2, "u2", 1).await.unwrap();

        let manual = list_manual(&db).await.unwrap();
        assert_eq!(manual.len(), 1);
        assert_eq!(manual[0].series_id, s2);
    }

    #[tokio::test]
    async fn cascade_delete_drops_link_with_series() {
        let db = fresh_db().await;
        let s = insert_series(&db, "A").await;
        upsert_auto(&db, &auto(s, "u1", Some(1.0), 1))
            .await
            .unwrap();
        series::Entity::delete_by_id(s).exec(&db).await.unwrap();
        assert!(get(&db, s).await.unwrap().is_none());
    }
}
