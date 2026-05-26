//! Series read/write helpers.

use anyhow::Result;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, QuerySelect, Statement,
};

use crate::entities::series;

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
                    series::Column::GenresJson,
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

// re-export the entity column/active-model surface for callers that need to
// build their own ActiveModel literals without depending on td-db internals.
pub use series::{ActiveModel, Column, Entity};
