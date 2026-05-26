use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for stmt in UP {
            db.execute_unprepared(stmt).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for stmt in DOWN {
            db.execute_unprepared(stmt).await?;
        }
        Ok(())
    }
}

// Genres and tags get their own normalized tables so the feed UI can filter
// by them (indexed lookup) and autocomplete from a canonical list. `series.
// genres_json` still gets written for one release as a fallback so a quick
// revert keeps detail responses unchanged; a follow-up migration drops the
// column once the UI reads exclusively from the join tables.
//
// The backfill at the bottom of `up()` uses `json_each` to lift any existing
// `series.genres_json` blobs into the new tables. Empty / null blobs are
// no-ops thanks to `WHERE ... IS NOT NULL` plus the LIKE shape filter.
const UP: &[&str] = &[
    "CREATE TABLE genres (
        id   INTEGER PRIMARY KEY,
        name TEXT NOT NULL COLLATE NOCASE,
        UNIQUE(name) ON CONFLICT IGNORE
    )",
    "CREATE TABLE tags (
        id   INTEGER PRIMARY KEY,
        name TEXT NOT NULL COLLATE NOCASE,
        UNIQUE(name) ON CONFLICT IGNORE
    )",
    "CREATE TABLE series_genres (
        series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
        genre_id  INTEGER NOT NULL REFERENCES genres(id) ON DELETE CASCADE,
        PRIMARY KEY (series_id, genre_id)
    )",
    "CREATE INDEX ix_series_genres_genre ON series_genres(genre_id)",
    "CREATE TABLE series_tags (
        series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
        tag_id    INTEGER NOT NULL REFERENCES tags(id)    ON DELETE CASCADE,
        PRIMARY KEY (series_id, tag_id)
    )",
    "CREATE INDEX ix_series_tags_tag ON series_tags(tag_id)",
    // Backfill genres from existing `series.genres_json` arrays. SQLite's
    // json_each yields one row per element; we trim whitespace and drop
    // empty entries before inserting. INSERT OR IGNORE relies on the
    // UNIQUE constraint on `genres.name`.
    "INSERT OR IGNORE INTO genres (name)
        SELECT DISTINCT TRIM(j.value)
        FROM series s, json_each(s.genres_json) j
        WHERE s.genres_json IS NOT NULL
          AND s.genres_json LIKE '[%]'
          AND TRIM(j.value) <> ''",
    "INSERT OR IGNORE INTO series_genres (series_id, genre_id)
        SELECT s.id, g.id
        FROM series s, json_each(s.genres_json) j
        JOIN genres g ON g.name = TRIM(j.value) COLLATE NOCASE
        WHERE s.genres_json IS NOT NULL
          AND s.genres_json LIKE '[%]'
          AND TRIM(j.value) <> ''",
];

const DOWN: &[&str] = &[
    "DROP INDEX IF EXISTS ix_series_tags_tag",
    "DROP TABLE IF EXISTS series_tags",
    "DROP INDEX IF EXISTS ix_series_genres_genre",
    "DROP TABLE IF EXISTS series_genres",
    "DROP TABLE IF EXISTS tags",
    "DROP TABLE IF EXISTS genres",
];
