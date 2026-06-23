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

// Add `description` as a third indexed column to the `series_fts` mirror so an
// optional, per-search toggle can widen free-text search beyond titles. The
// `description` column landed after the original FTS table, so the table never
// indexed it. external-content mode means we can drop and reinit the table +
// triggers without touching `series` data, then backfill from the content
// table (the local catalog is small, so this is cheap). We can't use the FTS5
// `'rebuild'` command because it reads columns named like the FTS columns
// (`title`, ...) while `series` names them `canonical_title` /
// `alternate_titles_json`, so we map them explicitly in an `INSERT ... SELECT`.
// The query path scopes MATCH to `{title alternate_titles}` when the toggle is
// off, so default search stays title-only.
const UP: &[&str] = &[
    "DROP TRIGGER IF EXISTS series_au",
    "DROP TRIGGER IF EXISTS series_ad",
    "DROP TRIGGER IF EXISTS series_ai",
    "DROP TABLE IF EXISTS series_fts",
    "CREATE VIRTUAL TABLE series_fts USING fts5(
        title,
        alternate_titles,
        description,
        content='series',
        content_rowid='id'
    )",
    "CREATE TRIGGER series_ai AFTER INSERT ON series BEGIN
        INSERT INTO series_fts(rowid, title, alternate_titles, description)
        VALUES (new.id, new.canonical_title, COALESCE(new.alternate_titles_json, ''), COALESCE(new.description, ''));
    END",
    "CREATE TRIGGER series_ad AFTER DELETE ON series BEGIN
        INSERT INTO series_fts(series_fts, rowid, title, alternate_titles, description)
        VALUES('delete', old.id, old.canonical_title, COALESCE(old.alternate_titles_json, ''), COALESCE(old.description, ''));
    END",
    "CREATE TRIGGER series_au AFTER UPDATE ON series BEGIN
        INSERT INTO series_fts(series_fts, rowid, title, alternate_titles, description)
        VALUES('delete', old.id, old.canonical_title, COALESCE(old.alternate_titles_json, ''), COALESCE(old.description, ''));
        INSERT INTO series_fts(rowid, title, alternate_titles, description)
        VALUES (new.id, new.canonical_title, COALESCE(new.alternate_titles_json, ''), COALESCE(new.description, ''));
    END",
    "INSERT INTO series_fts(rowid, title, alternate_titles, description)
        SELECT id, canonical_title, COALESCE(alternate_titles_json, ''), COALESCE(description, '')
        FROM series",
];

// Reverse to the original two-column (title + alternate_titles) form.
const DOWN: &[&str] = &[
    "DROP TRIGGER IF EXISTS series_au",
    "DROP TRIGGER IF EXISTS series_ad",
    "DROP TRIGGER IF EXISTS series_ai",
    "DROP TABLE IF EXISTS series_fts",
    "CREATE VIRTUAL TABLE series_fts USING fts5(
        title,
        alternate_titles,
        content='series',
        content_rowid='id'
    )",
    "CREATE TRIGGER series_ai AFTER INSERT ON series BEGIN
        INSERT INTO series_fts(rowid, title, alternate_titles)
        VALUES (new.id, new.canonical_title, COALESCE(new.alternate_titles_json, ''));
    END",
    "CREATE TRIGGER series_ad AFTER DELETE ON series BEGIN
        INSERT INTO series_fts(series_fts, rowid, title, alternate_titles)
        VALUES('delete', old.id, old.canonical_title, COALESCE(old.alternate_titles_json, ''));
    END",
    "CREATE TRIGGER series_au AFTER UPDATE ON series BEGIN
        INSERT INTO series_fts(series_fts, rowid, title, alternate_titles)
        VALUES('delete', old.id, old.canonical_title, COALESCE(old.alternate_titles_json, ''));
        INSERT INTO series_fts(rowid, title, alternate_titles)
        VALUES (new.id, new.canonical_title, COALESCE(new.alternate_titles_json, ''));
    END",
    "INSERT INTO series_fts(rowid, title, alternate_titles)
        SELECT id, canonical_title, COALESCE(alternate_titles_json, '')
        FROM series",
];
