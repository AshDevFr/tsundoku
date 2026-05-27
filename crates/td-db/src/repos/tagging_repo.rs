//! Normalized genre/tag repository: upsert a series's full genre + tag set
//! and read the canonical lists for filter autocomplete.
//!
//! Two write helpers (`set_series_genres`, `set_series_tags`) sync a series's
//! current set in one shot: each upserts canonical-list rows, then replaces
//! the join-table membership for that series. Both run in a transaction so
//! a partial failure can't leave the series with the wrong half-set.

use anyhow::Result;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    Set, Statement, TransactionTrait,
};

use crate::entities::{genres, series_genres, series_tags, tags};

#[derive(Debug, Clone, FromQueryResult, serde::Serialize, serde::Deserialize)]
pub struct NameUsage {
    pub name: String,
    pub series_count: i64,
}

/// Replace the genre membership for `series_id` with the supplied names.
/// Empty input clears the join table for the series.
pub async fn set_series_genres(
    db: &DatabaseConnection,
    series_id: i32,
    names: &[String],
) -> Result<()> {
    let txn = db.begin().await?;
    series_genres::Entity::delete_many()
        .filter(series_genres::Column::SeriesId.eq(series_id))
        .exec(&txn)
        .await?;
    let clean = normalize(names);
    if clean.is_empty() {
        txn.commit().await?;
        return Ok(());
    }
    upsert_genre_names(&txn, &clean).await?;
    let ids = resolve_genre_ids(&txn, &clean).await?;
    if !ids.is_empty() {
        let rows: Vec<series_genres::ActiveModel> = ids
            .into_iter()
            .map(|gid| series_genres::ActiveModel {
                series_id: Set(series_id),
                genre_id: Set(gid),
            })
            .collect();
        series_genres::Entity::insert_many(rows)
            .on_conflict(
                OnConflict::columns([
                    series_genres::Column::SeriesId,
                    series_genres::Column::GenreId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(&txn)
            .await?;
    }
    txn.commit().await?;
    Ok(())
}

/// Mirror of `set_series_genres` for tags.
pub async fn set_series_tags(
    db: &DatabaseConnection,
    series_id: i32,
    names: &[String],
) -> Result<()> {
    let txn = db.begin().await?;
    series_tags::Entity::delete_many()
        .filter(series_tags::Column::SeriesId.eq(series_id))
        .exec(&txn)
        .await?;
    let clean = normalize(names);
    if clean.is_empty() {
        txn.commit().await?;
        return Ok(());
    }
    upsert_tag_names(&txn, &clean).await?;
    let ids = resolve_tag_ids(&txn, &clean).await?;
    if !ids.is_empty() {
        let rows: Vec<series_tags::ActiveModel> = ids
            .into_iter()
            .map(|tid| series_tags::ActiveModel {
                series_id: Set(series_id),
                tag_id: Set(tid),
            })
            .collect();
        series_tags::Entity::insert_many(rows)
            .on_conflict(
                OnConflict::columns([series_tags::Column::SeriesId, series_tags::Column::TagId])
                    .do_nothing()
                    .to_owned(),
            )
            .exec_without_returning(&txn)
            .await?;
    }
    txn.commit().await?;
    Ok(())
}

/// Names + series-count for every genre that's been observed at least once.
/// Sorted by descending count, then name for deterministic order.
pub async fn list_genres_with_counts(db: &DatabaseConnection) -> Result<Vec<NameUsage>> {
    let backend = db.get_database_backend();
    let sql = "SELECT g.name AS name, COUNT(sg.series_id) AS series_count
               FROM genres g
               LEFT JOIN series_genres sg ON sg.genre_id = g.id
               GROUP BY g.id
               ORDER BY series_count DESC, name ASC";
    let stmt = Statement::from_sql_and_values(backend, sql, []);
    let rows = NameUsage::find_by_statement(stmt).all(db).await?;
    Ok(rows)
}

pub async fn list_tags_with_counts(db: &DatabaseConnection) -> Result<Vec<NameUsage>> {
    let backend = db.get_database_backend();
    let sql = "SELECT t.name AS name, COUNT(st.series_id) AS series_count
               FROM tags t
               LEFT JOIN series_tags st ON st.tag_id = t.id
               GROUP BY t.id
               ORDER BY series_count DESC, name ASC";
    let stmt = Statement::from_sql_and_values(backend, sql, []);
    let rows = NameUsage::find_by_statement(stmt).all(db).await?;
    Ok(rows)
}

/// Names attached to a single series, in canonical order. Empty when the
/// series has no genres or doesn't exist.
pub async fn list_genres_for_series(
    db: &DatabaseConnection,
    series_id: i32,
) -> Result<Vec<String>> {
    let backend = db.get_database_backend();
    let sql = "SELECT g.name AS name, 0 AS series_count
               FROM series_genres sg
               JOIN genres g ON g.id = sg.genre_id
               WHERE sg.series_id = ?1
               ORDER BY g.name ASC";
    let stmt = Statement::from_sql_and_values(backend, sql, [(series_id as i64).into()]);
    let rows = NameUsage::find_by_statement(stmt).all(db).await?;
    Ok(rows.into_iter().map(|r| r.name).collect())
}

pub async fn list_tags_for_series(db: &DatabaseConnection, series_id: i32) -> Result<Vec<String>> {
    let backend = db.get_database_backend();
    let sql = "SELECT t.name AS name, 0 AS series_count
               FROM series_tags st
               JOIN tags t ON t.id = st.tag_id
               WHERE st.series_id = ?1
               ORDER BY t.name ASC";
    let stmt = Statement::from_sql_and_values(backend, sql, [(series_id as i64).into()]);
    let rows = NameUsage::find_by_statement(stmt).all(db).await?;
    Ok(rows.into_iter().map(|r| r.name).collect())
}

#[derive(Debug, FromQueryResult)]
struct SeriesIdName {
    series_id: i32,
    name: String,
}

/// Batch fetch the genres for every series id in `series_ids`, returned as
/// a map keyed by series id. Series with no genres are omitted from the map.
/// One SELECT, used by the list endpoint to avoid N+1.
pub async fn genres_by_series_ids(
    db: &DatabaseConnection,
    series_ids: &[i32],
) -> Result<std::collections::HashMap<i32, Vec<String>>> {
    if series_ids.is_empty() {
        return Ok(Default::default());
    }
    let placeholders = vec!["?"; series_ids.len()].join(",");
    let sql = format!(
        "SELECT sg.series_id AS series_id, g.name AS name \
         FROM series_genres sg JOIN genres g ON g.id = sg.genre_id \
         WHERE sg.series_id IN ({placeholders}) \
         ORDER BY sg.series_id, g.name ASC"
    );
    let backend = db.get_database_backend();
    let values: Vec<sea_orm::Value> = series_ids.iter().map(|id| (*id as i64).into()).collect();
    let stmt = Statement::from_sql_and_values(backend, &sql, values);
    let rows = SeriesIdName::find_by_statement(stmt).all(db).await?;
    let mut out: std::collections::HashMap<i32, Vec<String>> = std::collections::HashMap::new();
    for r in rows {
        out.entry(r.series_id).or_default().push(r.name);
    }
    Ok(out)
}

/// Mirror of `genres_by_series_ids` for tags.
pub async fn tags_by_series_ids(
    db: &DatabaseConnection,
    series_ids: &[i32],
) -> Result<std::collections::HashMap<i32, Vec<String>>> {
    if series_ids.is_empty() {
        return Ok(Default::default());
    }
    let placeholders = vec!["?"; series_ids.len()].join(",");
    let sql = format!(
        "SELECT st.series_id AS series_id, t.name AS name \
         FROM series_tags st JOIN tags t ON t.id = st.tag_id \
         WHERE st.series_id IN ({placeholders}) \
         ORDER BY st.series_id, t.name ASC"
    );
    let backend = db.get_database_backend();
    let values: Vec<sea_orm::Value> = series_ids.iter().map(|id| (*id as i64).into()).collect();
    let stmt = Statement::from_sql_and_values(backend, &sql, values);
    let rows = SeriesIdName::find_by_statement(stmt).all(db).await?;
    let mut out: std::collections::HashMap<i32, Vec<String>> = std::collections::HashMap::new();
    for r in rows {
        out.entry(r.series_id).or_default().push(r.name);
    }
    Ok(out)
}

fn normalize(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = names
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    // Case-insensitive dedup so two casings of the same genre don't insert
    // twice (the UNIQUE on `name COLLATE NOCASE` would catch it, but dedup
    // up front saves a round trip).
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    out.retain(|s| seen.insert(s.to_lowercase()));
    out
}

async fn upsert_genre_names<C: ConnectionTrait>(db: &C, names: &[String]) -> Result<()> {
    let rows: Vec<genres::ActiveModel> = names
        .iter()
        .map(|name| genres::ActiveModel {
            id: sea_orm::NotSet,
            name: Set(name.clone()),
        })
        .collect();
    if rows.is_empty() {
        return Ok(());
    }
    genres::Entity::insert_many(rows)
        .on_conflict(
            OnConflict::column(genres::Column::Name)
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    Ok(())
}

async fn upsert_tag_names<C: ConnectionTrait>(db: &C, names: &[String]) -> Result<()> {
    let rows: Vec<tags::ActiveModel> = names
        .iter()
        .map(|name| tags::ActiveModel {
            id: sea_orm::NotSet,
            name: Set(name.clone()),
        })
        .collect();
    if rows.is_empty() {
        return Ok(());
    }
    tags::Entity::insert_many(rows)
        .on_conflict(
            OnConflict::column(tags::Column::Name)
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    Ok(())
}

async fn resolve_genre_ids<C: ConnectionTrait>(db: &C, names: &[String]) -> Result<Vec<i32>> {
    let rows = genres::Entity::find()
        .filter(genres::Column::Name.is_in(names.iter().cloned()))
        .all(db)
        .await?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}

async fn resolve_tag_ids<C: ConnectionTrait>(db: &C, names: &[String]) -> Result<Vec<i32>> {
    let rows = tags::Entity::find()
        .filter(tags::Column::Name.is_in(names.iter().cloned()))
        .all(db)
        .await?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{ActiveModelTrait, Database};

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    async fn seed_series(db: &DatabaseConnection, title: &str) -> i32 {
        use crate::entities::series;
        use sea_orm::Set;
        let now = chrono::Utc::now().timestamp();
        let model = series::ActiveModel {
            canonical_title: Set(title.into()),
            alternate_titles_json: Set(None),
            cover_url: Set(None),
            kind: Set(Some("manga".into())),
            status: Set(Some("ongoing".into())),
            year: Set(Some(2020)),
            metadata_json: Set(None),
            metadata_source: Set("api".into()),
            metadata_hash: Set(None),
            metadata_fetched_at: Set(now),
            first_seen_at: Set(now),
            last_release_at: Set(now),
            highest_volume: Set(None),
            highest_chapter: Set(None),
            owned: Set(0),
            ..Default::default()
        };
        model.insert(db).await.unwrap().id
    }

    #[tokio::test]
    async fn set_series_genres_inserts_canonical_rows_and_join_membership() {
        let db = fresh_db().await;
        let sid = seed_series(&db, "Test").await;
        set_series_genres(
            &db,
            sid,
            &[
                "Action".into(),
                "horror".into(),
                " action ".into(),
                "".into(),
            ],
        )
        .await
        .unwrap();

        // Canonical list deduped on case + whitespace.
        let names = list_genres_for_series(&db, sid).await.unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|n| n.eq_ignore_ascii_case("Action")));
        assert!(names.iter().any(|n| n.eq_ignore_ascii_case("horror")));
    }

    #[tokio::test]
    async fn set_series_genres_replaces_previous_membership() {
        let db = fresh_db().await;
        let sid = seed_series(&db, "Test").await;
        set_series_genres(&db, sid, &["Action".into(), "Horror".into()])
            .await
            .unwrap();
        set_series_genres(&db, sid, &["Comedy".into()])
            .await
            .unwrap();
        let names = list_genres_for_series(&db, sid).await.unwrap();
        assert_eq!(names, vec!["Comedy"]);
    }

    #[tokio::test]
    async fn list_genres_with_counts_reports_usage_descending() {
        let db = fresh_db().await;
        let a = seed_series(&db, "A").await;
        let b = seed_series(&db, "B").await;
        let c = seed_series(&db, "C").await;
        set_series_genres(&db, a, &["Action".into(), "Drama".into()])
            .await
            .unwrap();
        set_series_genres(&db, b, &["Action".into()]).await.unwrap();
        set_series_genres(&db, c, &["Action".into()]).await.unwrap();
        let rows = list_genres_with_counts(&db).await.unwrap();
        assert_eq!(rows.first().map(|r| r.name.as_str()), Some("Action"));
        assert_eq!(rows.first().map(|r| r.series_count), Some(3));
        assert!(
            rows.iter()
                .any(|r| r.name == "Drama" && r.series_count == 1)
        );
    }

    #[tokio::test]
    async fn set_series_tags_mirrors_genre_behavior() {
        let db = fresh_db().await;
        let sid = seed_series(&db, "T").await;
        set_series_tags(&db, sid, &["isekai".into(), "Magic".into()])
            .await
            .unwrap();
        let names = list_tags_for_series(&db, sid).await.unwrap();
        assert_eq!(names.len(), 2);
        let counts = list_tags_with_counts(&db).await.unwrap();
        assert_eq!(counts.len(), 2);
    }

    #[tokio::test]
    async fn empty_input_clears_membership_but_keeps_canonical_rows() {
        let db = fresh_db().await;
        let sid = seed_series(&db, "Empty").await;
        set_series_genres(&db, sid, &["Action".into()])
            .await
            .unwrap();
        set_series_genres(&db, sid, &[]).await.unwrap();
        assert!(list_genres_for_series(&db, sid).await.unwrap().is_empty());
        // Canonical row still there, count drops to 0.
        let rows = list_genres_with_counts(&db).await.unwrap();
        let action = rows.iter().find(|r| r.name == "Action").unwrap();
        assert_eq!(action.series_count, 0);
    }
}
