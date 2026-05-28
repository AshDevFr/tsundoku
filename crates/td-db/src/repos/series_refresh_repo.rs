//! Read-side helpers for the bulk series-metadata refresh job.
//!
//! The selection query is the contract between the cron tick and the
//! manual `POST /api/v1/series/refresh-all` endpoint: both pick the same
//! rows the same way, so a manual trigger doesn't double-up work the cron
//! is about to do (or vice versa). Manual rows
//! (`series.metadata_source = 'manual'`) are filtered out at the query
//! level *and* defended-in-depth by the
//! `allow_manual_overwrite` guard in `td_resolution::persist`; either
//! one alone would suffice but the pair makes accidents loud.

use anyhow::Result;
use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement};

/// One row's worth of state needed to drive a refresh. Returned in
/// `metadata_fetched_at`-ascending order so the oldest rows go first.
#[derive(Debug, Clone, FromQueryResult, PartialEq, Eq)]
pub struct StaleSeriesRow {
    pub series_id: i32,
    /// The active provider's external_id for this series, taken from
    /// `series_external_ids`. The caller passes this straight into
    /// `MetadataProvider::get`.
    pub external_id: String,
    /// Snapshot of the stored `series.metadata_hash`. The caller can
    /// short-circuit the actual UPDATE when the provider returns an
    /// identical payload, but the hash check also runs inside
    /// `upsert_series_from_metadata` so both paths are safe.
    pub metadata_hash: Option<String>,
    pub metadata_source: String,
    pub metadata_fetched_at: i64,
}

/// Pick the next `batch_size` series rows to refresh, oldest-first, that
/// are mapped to `active_provider` and whose `metadata_fetched_at` is
/// older than `now - min_age_seconds`. Manual rows are excluded.
///
/// `batch_size = 0` is legal and returns an empty vector (useful for
/// transient disabling without un-registering the cron).
pub async fn select_stale_for_active_provider(
    db: &DatabaseConnection,
    active_provider: &str,
    batch_size: u32,
    min_age_seconds: i64,
    now: i64,
) -> Result<Vec<StaleSeriesRow>> {
    if batch_size == 0 {
        return Ok(Vec::new());
    }
    let cutoff = now.saturating_sub(min_age_seconds.max(0));
    let backend = db.get_database_backend();
    // Inner join: a series with no mapping for the active provider can't
    // be refreshed against that provider anyway, so it's not eligible.
    let sql = "SELECT s.id              AS series_id,
                      e.external_id     AS external_id,
                      s.metadata_hash   AS metadata_hash,
                      s.metadata_source AS metadata_source,
                      s.metadata_fetched_at AS metadata_fetched_at
               FROM series s
               INNER JOIN series_external_ids e
                 ON e.series_id = s.id
                AND e.provider = ?1
               WHERE s.metadata_source != 'manual'
                 AND s.metadata_fetched_at < ?2
               ORDER BY s.metadata_fetched_at ASC
               LIMIT ?3";
    let stmt = Statement::from_sql_and_values(
        backend,
        sql,
        [
            active_provider.into(),
            cutoff.into(),
            (batch_size as i64).into(),
        ],
    );
    Ok(StaleSeriesRow::find_by_statement(stmt).all(db).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{series, series_external_ids};
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{ActiveValue::Set, Database, EntityTrait};

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    /// Insert a `series` row with explicit timestamps + source, returning
    /// the new id. Defaults match the resolver's insert shape.
    async fn insert_series(
        db: &DatabaseConnection,
        title: &str,
        source: &str,
        fetched_at: i64,
    ) -> i32 {
        let model = series::ActiveModel {
            canonical_title: Set(title.into()),
            metadata_source: Set(source.into()),
            metadata_hash: Set(Some(format!("h-{title}"))),
            metadata_fetched_at: Set(fetched_at),
            first_seen_at: Set(fetched_at),
            last_release_at: Set(fetched_at),
            owned: Set(0),
            ..Default::default()
        };
        series::Entity::insert(model)
            .exec_with_returning(db)
            .await
            .unwrap()
            .id
    }

    async fn map_external(
        db: &DatabaseConnection,
        series_id: i32,
        provider: &str,
        external_id: &str,
    ) {
        let m = series_external_ids::ActiveModel {
            provider: Set(provider.into()),
            external_id: Set(external_id.into()),
            series_id: Set(series_id),
            fetched_at: Set(0),
        };
        series_external_ids::Entity::insert(m)
            .exec(db)
            .await
            .unwrap();
    }

    /// 1 day in seconds for readable test fixtures.
    const DAY: i64 = 86_400;

    #[tokio::test]
    async fn selects_oldest_first_above_floor() {
        let db = fresh_db().await;
        // now = day 100. min_age = 7 days, so cutoff = day 93.
        let now = 100 * DAY;
        let min_age = 7 * DAY;

        let s_old = insert_series(&db, "old", "api", 80 * DAY).await; // eligible (oldest)
        let s_mid = insert_series(&db, "mid", "api", 90 * DAY).await; // eligible
        let s_recent = insert_series(&db, "recent", "api", 95 * DAY).await; // newer than cutoff
        map_external(&db, s_old, "mangabaka", "ext-old").await;
        map_external(&db, s_mid, "mangabaka", "ext-mid").await;
        map_external(&db, s_recent, "mangabaka", "ext-recent").await;

        let rows = select_stale_for_active_provider(&db, "mangabaka", 10, min_age, now)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "recent row excluded by min_age floor");
        assert_eq!(rows[0].external_id, "ext-old", "oldest first");
        assert_eq!(rows[1].external_id, "ext-mid");
    }

    #[tokio::test]
    async fn excludes_manual_rows() {
        let db = fresh_db().await;
        let now = 100 * DAY;
        // Manual row is the oldest, so if it were eligible it would come
        // first; instead it should be absent entirely.
        let s_manual = insert_series(&db, "manual-row", "manual", 40 * DAY).await;
        let s_api = insert_series(&db, "api-row", "api", 50 * DAY).await;
        let s_offline = insert_series(&db, "offline-row", "offline_cache", 60 * DAY).await;
        map_external(&db, s_manual, "mangabaka", "ext-manual").await;
        map_external(&db, s_api, "mangabaka", "ext-api").await;
        map_external(&db, s_offline, "mangabaka", "ext-offline").await;

        let rows = select_stale_for_active_provider(&db, "mangabaka", 10, 7 * DAY, now)
            .await
            .unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.external_id.as_str()).collect();
        assert_eq!(ids, vec!["ext-api", "ext-offline"]);
    }

    #[tokio::test]
    async fn excludes_series_without_mapping_for_active_provider() {
        let db = fresh_db().await;
        let now = 100 * DAY;
        let s_only_anilist = insert_series(&db, "anilist-only", "api", 50 * DAY).await;
        let s_both = insert_series(&db, "both", "api", 60 * DAY).await;
        map_external(&db, s_only_anilist, "anilist", "anilist-1").await;
        map_external(&db, s_both, "anilist", "anilist-2").await;
        map_external(&db, s_both, "mangabaka", "mb-2").await;

        let rows = select_stale_for_active_provider(&db, "mangabaka", 10, 7 * DAY, now)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].external_id, "mb-2");
    }

    #[tokio::test]
    async fn respects_batch_size_limit() {
        let db = fresh_db().await;
        let now = 100 * DAY;
        for i in 0..5 {
            let sid = insert_series(&db, &format!("s{i}"), "api", (10 + i) * DAY).await;
            map_external(&db, sid, "mangabaka", &format!("ext-{i}")).await;
        }
        let rows = select_stale_for_active_provider(&db, "mangabaka", 3, 7 * DAY, now)
            .await
            .unwrap();
        assert_eq!(rows.len(), 3);
        // Oldest three: ext-0 (10d), ext-1 (11d), ext-2 (12d).
        let ids: Vec<&str> = rows.iter().map(|r| r.external_id.as_str()).collect();
        assert_eq!(ids, vec!["ext-0", "ext-1", "ext-2"]);
    }

    #[tokio::test]
    async fn batch_size_zero_is_a_no_op() {
        let db = fresh_db().await;
        let sid = insert_series(&db, "x", "api", 0).await;
        map_external(&db, sid, "mangabaka", "ext").await;
        let rows = select_stale_for_active_provider(&db, "mangabaka", 0, 0, 100 * DAY)
            .await
            .unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn empty_when_no_rows_qualify() {
        let db = fresh_db().await;
        // All rows fresh relative to cutoff.
        let now = 100 * DAY;
        let sid = insert_series(&db, "fresh", "api", 99 * DAY).await;
        map_external(&db, sid, "mangabaka", "ext").await;
        let rows = select_stale_for_active_provider(&db, "mangabaka", 10, 7 * DAY, now)
            .await
            .unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn returns_hash_and_source_for_observation() {
        let db = fresh_db().await;
        let now = 100 * DAY;
        let sid = insert_series(&db, "rich", "offline_cache", 10 * DAY).await;
        map_external(&db, sid, "mangabaka", "ext-rich").await;
        let rows = select_stale_for_active_provider(&db, "mangabaka", 10, 7 * DAY, now)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.series_id, sid);
        assert_eq!(r.metadata_source, "offline_cache");
        assert_eq!(r.metadata_hash.as_deref(), Some("h-rich"));
        assert_eq!(r.metadata_fetched_at, 10 * DAY);
    }
}
