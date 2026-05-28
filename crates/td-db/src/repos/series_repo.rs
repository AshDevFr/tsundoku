//! Series read/write helpers.

use anyhow::Result;
use sea_orm::sea_query::{Expr, OnConflict, Query};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, FromQueryResult, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Statement, TransactionTrait,
};

use crate::entities::{series, series_external_ids};

/// Provenance string used in `series.metadata_source` for operator-authored
/// rows that have no provider behind them. Kept in the repo because every
/// bulk write needs to skip these (manual rows have no upstream to refetch
/// from, so a hash invalidation would do nothing useful).
const MANUAL_METADATA_SOURCE: &str = "manual";

/// Result of a bulk hash-invalidation call.
#[derive(Debug, Clone, Copy)]
pub struct InvalidateHashesOutcome {
    /// Number of rows whose `metadata_hash` was cleared.
    pub invalidated: u64,
    /// Number of rows that matched the scope but were skipped because
    /// `metadata_source = 'manual'`. Reported for operator transparency:
    /// "you have N manual rows we left alone."
    pub skipped_manual: u64,
}

pub use series::Model;

pub async fn upsert(db: &DatabaseConnection, model: series::ActiveModel) -> Result<series::Model> {
    let inserted = series::Entity::insert(model)
        .on_conflict(
            OnConflict::column(series::Column::Id)
                .update_columns([
                    series::Column::CanonicalTitle,
                    series::Column::AlternateTitlesJson,
                    series::Column::CoverUrl,
                    series::Column::Kind,
                    series::Column::Status,
                    series::Column::Year,
                    series::Column::MetadataJson,
                    series::Column::MetadataSource,
                    series::Column::MetadataHash,
                    series::Column::MetadataFetchedAt,
                    series::Column::LastReleaseAt,
                    series::Column::HighestVolume,
                    series::Column::HighestChapter,
                ])
                .to_owned(),
        )
        .exec_with_returning(db)
        .await?;
    Ok(inserted)
}

/// Insert a brand-new series row, returning the persisted model (with its
/// auto-assigned `id`). Unlike [`upsert`], this never updates an existing
/// row: the caller is creating a fresh series (e.g. an operator-authored
/// manual series with no provider mapping), so `id` must be left unset.
pub async fn create(db: &DatabaseConnection, model: series::ActiveModel) -> Result<series::Model> {
    Ok(series::Entity::insert(model)
        .exec_with_returning(db)
        .await?)
}

pub async fn find_by_id(db: &DatabaseConnection, id: i32) -> Result<Option<Model>> {
    Ok(series::Entity::find_by_id(id).one(db).await?)
}

pub async fn recent(db: &DatabaseConnection, limit: u64) -> Result<Vec<Model>> {
    Ok(series::Entity::find()
        .order_by_desc(series::Column::LastReleaseAt)
        .limit(limit)
        .all(db)
        .await?)
}

/// Find the series whose `metadata_hash` matches the given hash, if any.
/// Used by the resolver to skip a write when an incoming metadata payload
/// is byte-for-byte identical to what's already on the row.
pub async fn find_by_hash(db: &DatabaseConnection, hash: &str) -> Result<Option<Model>> {
    Ok(series::Entity::find()
        .filter(series::Column::MetadataHash.eq(hash))
        .one(db)
        .await?)
}

/// FTS5 match against `series_fts`. Returns matching series rows ordered by
/// FTS relevance (best first). The caller supplies a raw FTS5 MATCH expression
/// (e.g. a quoted phrase or column-prefixed term); the function does not
/// escape the input.
pub async fn search_fts(
    db: &DatabaseConnection,
    match_expr: &str,
    limit: u64,
) -> Result<Vec<Model>> {
    let backend = db.get_database_backend();
    let sql = "SELECT s.*
               FROM series s
               JOIN series_fts f ON f.rowid = s.id
               WHERE series_fts MATCH ?1
               ORDER BY rank
               LIMIT ?2";
    let stmt =
        Statement::from_sql_and_values(backend, sql, [match_expr.into(), (limit as i64).into()]);
    let rows = Model::find_by_statement(stmt).all(db).await?;
    Ok(rows)
}

/// Clear `metadata_hash` for every provider-backed series row, so the
/// next provider refresh tick rewrites them instead of short-circuiting
/// on a hash match.
///
/// `provider` scopes the operation:
/// - `None`: every non-manual row is cleared (today this is equivalent
///   to "every series resolved by any provider", since v1 has one).
/// - `Some(id)`: only rows that have a `series_external_ids` entry for
///   that provider id are cleared.
///
/// Manual rows (`metadata_source = 'manual'`) are always skipped — they
/// have no provider behind them, so the hash has no meaning.
///
/// The count + UPDATE run in one transaction so the reported
/// `skipped_manual` is consistent with what the UPDATE actually
/// considered, even if a write lands concurrently. Returns
/// `(invalidated, skipped_manual)`.
pub async fn invalidate_metadata_hashes(
    db: &DatabaseConnection,
    provider: Option<&str>,
) -> Result<InvalidateHashesOutcome> {
    let txn = db.begin().await?;

    let mut manual_q =
        series::Entity::find().filter(series::Column::MetadataSource.eq(MANUAL_METADATA_SOURCE));
    if let Some(p) = provider {
        manual_q = manual_q.filter(provider_filter(p));
    }
    let skipped_manual = manual_q.count(&txn).await?;

    let mut update = series::Entity::update_many()
        .col_expr(
            series::Column::MetadataHash,
            Expr::value(Option::<String>::None),
        )
        .filter(series::Column::MetadataSource.ne(MANUAL_METADATA_SOURCE))
        // Avoid pointless writes on rows that already have a NULL hash
        // (a fresh series with no prior refresh, or one already cleared
        // by an earlier call). Keeps `rows_affected` honest.
        .filter(series::Column::MetadataHash.is_not_null());
    if let Some(p) = provider {
        update = update.filter(provider_filter(p));
    }
    let invalidated = update.exec(&txn).await?.rows_affected;

    txn.commit().await?;
    Ok(InvalidateHashesOutcome {
        invalidated,
        skipped_manual,
    })
}

/// `series.id IN (SELECT series_id FROM series_external_ids WHERE provider = ?)`.
/// Factored out so the count and the UPDATE apply the exact same scope.
fn provider_filter(provider: &str) -> sea_orm::sea_query::SimpleExpr {
    series::Column::Id.in_subquery(
        Query::select()
            .column(series_external_ids::Column::SeriesId)
            .from(series_external_ids::Entity)
            .and_where(series_external_ids::Column::Provider.eq(provider))
            .to_owned(),
    )
}

// re-export the entity column/active-model surface for callers that need to
// build their own ActiveModel literals without depending on td-db internals.
pub use series::{ActiveModel, Column, Entity};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{Database, Set};

    async fn fresh() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    async fn seed(
        db: &DatabaseConnection,
        title: &str,
        metadata_source: &str,
        metadata_hash: Option<&str>,
    ) -> i32 {
        let now = Utc::now().timestamp();
        let model = series::ActiveModel {
            canonical_title: Set(title.into()),
            metadata_source: Set(metadata_source.into()),
            metadata_hash: Set(metadata_hash.map(str::to_owned)),
            metadata_fetched_at: Set(now),
            first_seen_at: Set(now),
            last_release_at: Set(now),
            owned: Set(0),
            ..Default::default()
        };
        series::Entity::insert(model)
            .exec_with_returning(db)
            .await
            .unwrap()
            .id
    }

    async fn link(db: &DatabaseConnection, series_id: i32, provider: &str, external_id: &str) {
        let now = Utc::now().timestamp();
        series_external_ids::Entity::insert(series_external_ids::ActiveModel {
            provider: Set(provider.into()),
            external_id: Set(external_id.into()),
            series_id: Set(series_id),
            fetched_at: Set(now),
        })
        .exec(db)
        .await
        .unwrap();
    }

    async fn hash_of(db: &DatabaseConnection, id: i32) -> Option<String> {
        series::Entity::find_by_id(id)
            .one(db)
            .await
            .unwrap()
            .unwrap()
            .metadata_hash
    }

    #[tokio::test]
    async fn clears_non_manual_hashes_and_skips_manual_rows() {
        let db = fresh().await;
        let a = seed(&db, "A", "api", Some("hash-a")).await;
        let b = seed(&db, "B", "offline_cache", Some("hash-b")).await;
        let manual = seed(&db, "M", "manual", Some("hash-m")).await;

        let out = invalidate_metadata_hashes(&db, None).await.unwrap();
        assert_eq!(out.invalidated, 2);
        assert_eq!(out.skipped_manual, 1);

        assert_eq!(hash_of(&db, a).await, None);
        assert_eq!(hash_of(&db, b).await, None);
        assert_eq!(hash_of(&db, manual).await, Some("hash-m".into()));
    }

    #[tokio::test]
    async fn ignores_rows_with_already_null_hash() {
        let db = fresh().await;
        let _stale = seed(&db, "stale", "api", None).await;
        let live = seed(&db, "live", "api", Some("hash-live")).await;

        let out = invalidate_metadata_hashes(&db, None).await.unwrap();
        assert_eq!(out.invalidated, 1, "only the row with a hash counts");
        assert_eq!(hash_of(&db, live).await, None);
    }

    #[tokio::test]
    async fn provider_filter_scopes_to_rows_with_matching_external_id() {
        let db = fresh().await;
        let mb = seed(&db, "MB-backed", "api", Some("hash-mb")).await;
        link(&db, mb, "mangabaka", "1").await;
        let other = seed(&db, "Other-backed", "api", Some("hash-other")).await;
        link(&db, other, "anilist", "999").await;

        let out = invalidate_metadata_hashes(&db, Some("mangabaka"))
            .await
            .unwrap();
        assert_eq!(out.invalidated, 1);
        assert_eq!(out.skipped_manual, 0);

        assert_eq!(hash_of(&db, mb).await, None);
        assert_eq!(
            hash_of(&db, other).await,
            Some("hash-other".into()),
            "anilist-backed row stays untouched when scoped to mangabaka",
        );
    }
}
