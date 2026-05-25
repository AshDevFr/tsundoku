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

// SQLite-only schema. Timestamps are Unix epoch seconds stored as INTEGER.
// `series.mangabaka_id` is INTEGER PRIMARY KEY so it aliases the rowid, which
// is required for the FTS5 contentless-rowid mirror of the title columns.
const UP: &[&str] = &[
    "CREATE TABLE series (
        mangabaka_id            INTEGER PRIMARY KEY,
        title                   TEXT NOT NULL,
        alternate_titles_json   TEXT,
        cover_url               TEXT,
        type                    TEXT,
        status                  TEXT,
        year                    INTEGER,
        genres_json             TEXT,
        metadata_json           TEXT,
        metadata_source         TEXT NOT NULL,
        metadata_fetched_at     INTEGER NOT NULL,
        first_seen_at           INTEGER NOT NULL,
        last_release_at         INTEGER NOT NULL,
        highest_volume          REAL,
        highest_chapter         REAL,
        owned                   INTEGER NOT NULL DEFAULT 0
    )",
    "CREATE INDEX idx_series_last_release ON series(last_release_at DESC)",
    "CREATE INDEX idx_series_type ON series(type)",
    "CREATE TABLE releases (
        id                      TEXT PRIMARY KEY,
        source_kind             TEXT NOT NULL,
        source_name             TEXT NOT NULL,
        external_id             TEXT NOT NULL,
        title                   TEXT NOT NULL,
        link                    TEXT NOT NULL,
        magnet                  TEXT,
        torrent_url             TEXT,
        ddl_url                 TEXT,
        info_hash               TEXT,
        size_bytes              INTEGER,
        files_json              TEXT,
        description_html        TEXT,
        extracted_links_json    TEXT,
        posted_at               INTEGER NOT NULL,
        observed_at             INTEGER NOT NULL,
        series_id               INTEGER,
        resolution_path         TEXT,
        resolution_confidence   REAL,
        resolution_status       TEXT NOT NULL,
        resolution_attempts     INTEGER NOT NULL DEFAULT 0,
        last_resolve_attempt_at INTEGER,
        volume_span_json        TEXT,
        chapter_span_json       TEXT,
        UNIQUE (source_kind, external_id),
        UNIQUE (link),
        FOREIGN KEY (series_id) REFERENCES series(mangabaka_id)
    )",
    "CREATE INDEX idx_releases_status ON releases(resolution_status)",
    "CREATE INDEX idx_releases_series ON releases(series_id)",
    "CREATE INDEX idx_releases_observed ON releases(observed_at DESC)",
    "CREATE INDEX idx_releases_source ON releases(source_kind, source_name)",
    "CREATE TABLE release_formats (
        release_id  TEXT NOT NULL,
        format      TEXT NOT NULL,
        PRIMARY KEY (release_id, format),
        FOREIGN KEY (release_id) REFERENCES releases(id) ON DELETE CASCADE
    )",
    "CREATE TABLE source_state (
        source_kind         TEXT NOT NULL,
        source_name         TEXT NOT NULL,
        etag                TEXT,
        cursor              TEXT,
        last_polled_at      INTEGER,
        last_success_at     INTEGER,
        last_error          TEXT,
        last_summary        TEXT,
        PRIMARY KEY (source_kind, source_name)
    )",
    // `id` is a sea-orm-friendly surrogate key; the underlying table is
    // conceptually an append-only log of dump refreshes.
    "CREATE TABLE mangabaka_offline (
        id              INTEGER PRIMARY KEY AUTOINCREMENT,
        fetched_at      INTEGER NOT NULL,
        dump_version    TEXT,
        record_count    INTEGER,
        source_url      TEXT
    )",
    "CREATE TABLE review_candidates (
        release_id      TEXT NOT NULL,
        mangabaka_id    INTEGER NOT NULL,
        score           REAL NOT NULL,
        reason          TEXT,
        PRIMARY KEY (release_id, mangabaka_id),
        FOREIGN KEY (release_id) REFERENCES releases(id) ON DELETE CASCADE
    )",
    // FTS5 mirror of series.title + alternate_titles_json, keyed on the
    // series rowid (== mangabaka_id). External-content mode keeps the FTS
    // table small and lets us drop/reinit without touching series data.
    "CREATE VIRTUAL TABLE series_fts USING fts5(
        title,
        alternate_titles,
        content='series',
        content_rowid='mangabaka_id'
    )",
    "CREATE TRIGGER series_ai AFTER INSERT ON series BEGIN
        INSERT INTO series_fts(rowid, title, alternate_titles)
        VALUES (new.mangabaka_id, new.title, COALESCE(new.alternate_titles_json, ''));
    END",
    "CREATE TRIGGER series_ad AFTER DELETE ON series BEGIN
        INSERT INTO series_fts(series_fts, rowid, title, alternate_titles)
        VALUES('delete', old.mangabaka_id, old.title, COALESCE(old.alternate_titles_json, ''));
    END",
    "CREATE TRIGGER series_au AFTER UPDATE ON series BEGIN
        INSERT INTO series_fts(series_fts, rowid, title, alternate_titles)
        VALUES('delete', old.mangabaka_id, old.title, COALESCE(old.alternate_titles_json, ''));
        INSERT INTO series_fts(rowid, title, alternate_titles)
        VALUES (new.mangabaka_id, new.title, COALESCE(new.alternate_titles_json, ''));
    END",
];

const DOWN: &[&str] = &[
    "DROP TRIGGER IF EXISTS series_au",
    "DROP TRIGGER IF EXISTS series_ad",
    "DROP TRIGGER IF EXISTS series_ai",
    "DROP TABLE IF EXISTS series_fts",
    "DROP TABLE IF EXISTS review_candidates",
    "DROP TABLE IF EXISTS mangabaka_offline",
    "DROP TABLE IF EXISTS source_state",
    "DROP TABLE IF EXISTS release_formats",
    "DROP TABLE IF EXISTS releases",
    "DROP TABLE IF EXISTS series",
];
