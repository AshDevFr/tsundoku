//! Post-extraction setup: add the indexes and FTS5 virtual table that
//! [`super::store::OfflineStore`] expects.
//!
//! The dump as published has no indexes on the source-id columns and no
//! FTS table. Adding them once per refresh costs a few minutes on 585k
//! rows but turns a sequential scan into an index lookup for every
//! foreign-id resolution. Run on the extracted file *before* renaming it
//! into the live `series.sqlite` slot.

use std::path::Path;

use anyhow::{Context, Result};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};

const INDEXES: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_series_anilist \
        ON series(source_anilist_id) WHERE source_anilist_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_series_mal \
        ON series(source_my_anime_list_id) WHERE source_my_anime_list_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_series_mangaupdates \
        ON series(source_manga_updates_id) WHERE source_manga_updates_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_series_kitsu \
        ON series(source_kitsu_id) WHERE source_kitsu_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_series_shikimori \
        ON series(source_shikimori_id) WHERE source_shikimori_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_series_anime_planet \
        ON series(source_anime_planet_id) WHERE source_anime_planet_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_series_ann \
        ON series(source_anime_news_network_id) WHERE source_anime_news_network_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_series_state ON series(state)",
];

/// Build indexes + FTS5 mirror on the extracted dump. Idempotent: a dump
/// that already went through `prepare` once is detected via the FTS5 table
/// presence and skipped.
pub async fn prepare(dump_path: impl AsRef<Path>) -> Result<()> {
    let path = dump_path.as_ref();
    if !path.exists() {
        anyhow::bail!("dump file not found at {}", path.display());
    }
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let mut opts = ConnectOptions::new(&url);
    opts.max_connections(1).sqlx_logging(false);
    let db = Database::connect(opts)
        .await
        .with_context(|| format!("opening dump for setup {}", path.display()))?;

    let backend = db.get_database_backend();

    // Skip re-running if FTS already exists.
    let fts_check = Statement::from_string(
        backend,
        "SELECT name FROM sqlite_master WHERE type='table' AND name='series_title_fts'".to_string(),
    );
    if db.query_one(fts_check).await?.is_some() {
        tracing::debug!("dump already prepared; skipping setup");
        return Ok(());
    }

    tracing::info!("preparing dump (indexes + FTS5); may take a few minutes");

    for stmt in INDEXES {
        db.execute(Statement::from_string(backend, (*stmt).to_string()))
            .await
            .with_context(|| format!("creating index via {stmt}"))?;
    }

    // Contentless FTS5: the dump stays unchanged, FTS5 stores only the
    // tokens. tokenize='unicode61 remove_diacritics 2' folds Latin
    // diacritics so accented romanizations match the bare ASCII query.
    db.execute(Statement::from_string(
        backend,
        "CREATE VIRTUAL TABLE series_title_fts USING fts5(
            title,
            native_title,
            romanized_title,
            alternate_titles,
            tokenize = 'unicode61 remove_diacritics 2'
        )"
        .to_string(),
    ))
    .await
    .context("creating series_title_fts")?;

    // Populate from active rows only. `titles` is a JSON array of
    // `{title, language, traits, is_primary, note}`. We extract every
    // contained title (excluding the canonical title to avoid duplicate
    // tokens) and stuff the lot into the `alternate_titles` FTS column.
    db.execute(Statement::from_string(
        backend,
        "INSERT INTO series_title_fts(rowid, title, native_title, romanized_title, alternate_titles)
         SELECT
             s.id,
             COALESCE(s.title, ''),
             COALESCE(s.native_title, ''),
             COALESCE(s.romanized_title, ''),
             COALESCE(
                 (SELECT GROUP_CONCAT(t.value, ' ')
                  FROM json_each(s.titles) AS j,
                       (SELECT json_extract(j.value, '$.title') AS value FROM json_each(s.titles) AS j) AS t
                  WHERE t.value IS NOT NULL AND t.value <> COALESCE(s.title, '')),
                 ''
             )
         FROM series s
         WHERE s.state = 'active' OR s.state IS NULL"
            .to_string(),
    ))
    .await
    .context("populating series_title_fts from active rows")?;

    // Drop any reference to the connection so the file is closed before
    // OfflineStore opens it RO.
    drop(db);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, Statement};
    use tempfile::TempDir;

    async fn make_dump(dir: &TempDir) -> std::path::PathBuf {
        let path = dir.path().join("series.sqlite");
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let db = Database::connect(&url).await.unwrap();
        let backend = db.get_database_backend();
        db.execute(Statement::from_string(
            backend,
            "CREATE TABLE series (
                id INTEGER PRIMARY KEY,
                title TEXT,
                native_title TEXT,
                romanized_title TEXT,
                titles TEXT,
                state TEXT,
                source_anilist_id INTEGER,
                source_my_anime_list_id INTEGER,
                source_manga_updates_id TEXT,
                source_kitsu_id INTEGER,
                source_shikimori_id INTEGER,
                source_anime_planet_id TEXT,
                source_anime_news_network_id INTEGER
            )"
            .to_string(),
        ))
        .await
        .unwrap();
        db.execute(Statement::from_string(
            backend,
            "INSERT INTO series (id, title, native_title, titles, state, source_anilist_id, source_manga_updates_id)
             VALUES (1, 'Berserk', 'ベルセルク', '[{\"title\":\"Berserk: Black Swordsman\"}]', 'active', 33, 'berserk-1')"
                .to_string(),
        ))
        .await
        .unwrap();
        db.execute(Statement::from_string(
            backend,
            "INSERT INTO series (id, title, state) VALUES (2, 'Merged Row', 'merged')".to_string(),
        ))
        .await
        .unwrap();
        drop(db);
        path
    }

    #[tokio::test]
    async fn prepare_builds_fts_and_populates_active_rows() {
        let dir = TempDir::new().unwrap();
        let path = make_dump(&dir).await;
        prepare(&path).await.unwrap();

        // Re-open RO and check FTS works.
        let db = Database::connect(format!("sqlite://{}?mode=ro", path.display()))
            .await
            .unwrap();
        let backend = db.get_database_backend();

        // FTS5 finds the canonical title.
        let rows = db
            .query_all(Statement::from_sql_and_values(
                backend,
                "SELECT rowid FROM series_title_fts WHERE series_title_fts MATCH ?1",
                ["\"Berserk\"".into()],
            ))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "FTS should match canonical title");

        // FTS5 finds an alternate title from the titles JSON array.
        let rows = db
            .query_all(Statement::from_sql_and_values(
                backend,
                "SELECT rowid FROM series_title_fts WHERE series_title_fts MATCH ?1",
                ["\"Swordsman\"".into()],
            ))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "FTS should match alternate from titles JSON");

        // Merged row excluded.
        let rows = db
            .query_all(Statement::from_sql_and_values(
                backend,
                "SELECT rowid FROM series_title_fts WHERE series_title_fts MATCH ?1",
                ["\"Merged\"".into()],
            ))
            .await
            .unwrap();
        assert!(rows.is_empty(), "merged rows must not appear in FTS");
    }

    #[tokio::test]
    async fn prepare_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = make_dump(&dir).await;
        prepare(&path).await.unwrap();
        prepare(&path).await.unwrap(); // second call must be a no-op
    }
}
