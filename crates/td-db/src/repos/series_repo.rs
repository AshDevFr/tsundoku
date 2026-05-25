//! Series read/write helpers.

use anyhow::Result;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryOrder, Statement,
};

use crate::entities::series;

pub use series::Model;

pub async fn upsert(db: &DatabaseConnection, model: series::ActiveModel) -> Result<()> {
    series::Entity::insert(model)
        .on_conflict(
            OnConflict::column(series::Column::MangabakaId)
                .update_columns([
                    series::Column::Title,
                    series::Column::AlternateTitlesJson,
                    series::Column::CoverUrl,
                    series::Column::Kind,
                    series::Column::Status,
                    series::Column::Year,
                    series::Column::GenresJson,
                    series::Column::MetadataJson,
                    series::Column::MetadataSource,
                    series::Column::MetadataFetchedAt,
                    series::Column::LastReleaseAt,
                    series::Column::HighestVolume,
                    series::Column::HighestChapter,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
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
               JOIN series_fts f ON f.rowid = s.mangabaka_id
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

// The auto-trait import that `limit()` needs.
use sea_orm::QuerySelect;
