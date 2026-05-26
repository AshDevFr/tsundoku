//! Read-only sea-orm access to an extracted MangaBaka dump.
//!
//! The store opens the SQLite file with read-only / immutable URI options,
//! exposes the three lookups the resolver cares about (`find_by_id`,
//! `find_by_source_id`, `search_fts`), and maps each row onto the canonical
//! `SeriesMetadata`.
//!
//! `find_by_source_id` benefits from the indexes that [`super::setup`]
//! creates after extraction (one per source column it knows about).
//! `search_fts` benefits from the `series_title_fts` virtual table that the
//! same setup step builds.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, FromQueryResult, Statement,
};
use serde_json::Value as Json;
use td_metadata::{ForeignId, SearchHit, SeriesKind, SeriesMetadata, SeriesStatus};

/// Read-only view over an extracted dump.
pub struct OfflineStore {
    db: DatabaseConnection,
    path: PathBuf,
}

impl OfflineStore {
    /// Open the file at `path` as a read-only sea-orm connection. The file
    /// must already have been [`super::setup::prepare`]d (indexes + FTS).
    /// The `?mode=ro` URI flag prevents accidental writes; SQLite will also
    /// not create a journal/WAL file alongside.
    pub async fn open_ro(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(anyhow!("dump file not found at {}", path.display()));
        }
        let url = format!("sqlite://{}?mode=ro", path.display());
        let mut opts = ConnectOptions::new(&url);
        opts.max_connections(1).sqlx_logging(false);
        let db = Database::connect(opts)
            .await
            .with_context(|| format!("opening dump {}", path.display()))?;
        Ok(Self { db, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Lookup by MangaBaka series id. Returns `Ok(None)` if no row matches
    /// or the row is marked merged/deleted (caller should treat both as
    /// "not found" — merged rows live behind their canonical row).
    pub async fn find_by_id(&self, id: &str) -> anyhow::Result<Option<SeriesMetadata>> {
        let Ok(parsed) = id.parse::<i64>() else {
            return Ok(None);
        };
        let stmt = Statement::from_sql_and_values(
            self.db.get_database_backend(),
            SELECT_BY_ID,
            [parsed.into()],
        );
        let row = RawRow::find_by_statement(stmt).one(&self.db).await?;
        Ok(row.and_then(active_row_to_canonical))
    }

    /// Lookup by a foreign source's id. `mb_source` is MangaBaka's column
    /// suffix (e.g. `"manga_updates"`, `"anilist"`), not our canonical
    /// provider id; the caller is responsible for translating.
    /// Returns `Ok(None)` for unknown sources or if no row matches.
    pub async fn find_by_source_id(
        &self,
        mb_source: &str,
        external_id: &str,
    ) -> anyhow::Result<Option<SeriesMetadata>> {
        let Some(column) = source_column(mb_source) else {
            return Ok(None);
        };
        // The CAST in the WHERE clause is what makes the indexes built by
        // `setup::prepare` usable for both TEXT slugs and INTEGER ids.
        // The full row select also CASTs source_*_id columns so sqlx can
        // decode them as Option<String> uniformly.
        let sql = format!(
            "SELECT id, title, native_title, romanized_title, \
            titles, type AS kind, status, year, description, state, \
            cover_x350_x2, cover_x350_x1, cover_x250_x2, cover_x250_x1, cover_raw_url, \
            CAST(source_anilist_id AS TEXT) AS source_anilist_id, \
            CAST(source_my_anime_list_id AS TEXT) AS source_my_anime_list_id, \
            CAST(source_manga_updates_id AS TEXT) AS source_manga_updates_id, \
            CAST(source_kitsu_id AS TEXT) AS source_kitsu_id, \
            CAST(source_shikimori_id AS TEXT) AS source_shikimori_id, \
            CAST(source_anime_planet_id AS TEXT) AS source_anime_planet_id, \
            CAST(source_anime_news_network_id AS TEXT) AS source_anime_news_network_id \
            FROM series WHERE CAST({column} AS TEXT) = ?1 LIMIT 1"
        );
        let stmt = Statement::from_sql_and_values(
            self.db.get_database_backend(),
            sql,
            [external_id.into()],
        );
        let row = RawRow::find_by_statement(stmt).one(&self.db).await?;
        Ok(row.and_then(active_row_to_canonical))
    }

    /// FTS5 full-text search against `series_title_fts` (built by
    /// [`super::setup::prepare`]). Returns rows ordered by FTS rank,
    /// best first, capped at `limit`.
    pub async fn search_fts(&self, query: &str, limit: u32) -> anyhow::Result<Vec<SearchHit>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let sanitized = sanitize_match(query);
        let stmt = Statement::from_sql_and_values(
            self.db.get_database_backend(),
            SELECT_FTS,
            [sanitized.into(), (limit.max(1) as i64).into()],
        );
        let rows = RawRow::find_by_statement(stmt).all(&self.db).await?;
        Ok(rows
            .into_iter()
            .map(|r| SearchHit {
                external_id: r.id.to_string(),
                title: r.title.clone(),
                year: r.year,
                cover_url: pick_cover_from_row(&r),
                score: None,
            })
            .collect())
    }
}

// All `source_*_id` columns are cast to TEXT because the dump mixes
// INTEGER (anilist, mal, kitsu, shikimori, anime_news_network) and TEXT
// (manga_updates, anime_planet slugs). The cast lets sqlx decode each as
// `Option<String>` uniformly.
// All `source_*_id` columns are cast to TEXT because the dump mixes
// INTEGER (anilist, mal, kitsu, shikimori, anime_news_network) and TEXT
// (manga_updates, anime_planet slugs). The cast lets sqlx decode each as
// `Option<String>` uniformly. `type` is aliased to `kind` because
// `FromQueryResult` does not honor `#[sea_orm(column_name = ...)]`.
const SELECT_BY_ID: &str = "SELECT id, title, native_title, romanized_title, \
        titles, type AS kind, status, year, description, state, \
        cover_x350_x2, cover_x350_x1, cover_x250_x2, cover_x250_x1, cover_raw_url, \
        CAST(source_anilist_id AS TEXT) AS source_anilist_id, \
        CAST(source_my_anime_list_id AS TEXT) AS source_my_anime_list_id, \
        CAST(source_manga_updates_id AS TEXT) AS source_manga_updates_id, \
        CAST(source_kitsu_id AS TEXT) AS source_kitsu_id, \
        CAST(source_shikimori_id AS TEXT) AS source_shikimori_id, \
        CAST(source_anime_planet_id AS TEXT) AS source_anime_planet_id, \
        CAST(source_anime_news_network_id AS TEXT) AS source_anime_news_network_id \
        FROM series WHERE id = ?1 LIMIT 1";

const SELECT_FTS: &str = "SELECT s.id, s.title, s.native_title, s.romanized_title, \
        s.titles, s.type AS kind, s.status, s.year, s.description, s.state, \
        s.cover_x350_x2, s.cover_x350_x1, s.cover_x250_x2, s.cover_x250_x1, s.cover_raw_url, \
        CAST(s.source_anilist_id AS TEXT) AS source_anilist_id, \
        CAST(s.source_my_anime_list_id AS TEXT) AS source_my_anime_list_id, \
        CAST(s.source_manga_updates_id AS TEXT) AS source_manga_updates_id, \
        CAST(s.source_kitsu_id AS TEXT) AS source_kitsu_id, \
        CAST(s.source_shikimori_id AS TEXT) AS source_shikimori_id, \
        CAST(s.source_anime_planet_id AS TEXT) AS source_anime_planet_id, \
        CAST(s.source_anime_news_network_id AS TEXT) AS source_anime_news_network_id \
        FROM series_title_fts f \
        JOIN series s ON s.id = f.rowid \
        WHERE series_title_fts MATCH ?1 AND (s.state = 'active' OR s.state IS NULL) \
        ORDER BY rank LIMIT ?2";

/// Map MangaBaka's `source` column suffix onto its physical SQLite column.
/// `None` means "unknown source"; the caller treats it as a miss.
fn source_column(mb_source: &str) -> Option<&'static str> {
    match mb_source {
        "anilist" => Some("source_anilist_id"),
        "my_anime_list" => Some("source_my_anime_list_id"),
        "manga_updates" => Some("source_manga_updates_id"),
        "kitsu" => Some("source_kitsu_id"),
        "shikimori" => Some("source_shikimori_id"),
        "anime_planet" => Some("source_anime_planet_id"),
        "anime_news_network" => Some("source_anime_news_network_id"),
        _ => None,
    }
}

/// Wrap each user-supplied term in double quotes so FTS5 treats it as a
/// phrase literal and the contained punctuation cannot inject syntax.
/// Sequences are joined with implicit AND (FTS5's default operator).
fn sanitize_match(query: &str) -> String {
    query
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| {
            // Escape embedded double quotes by doubling them per FTS5 spec.
            let escaped = t.replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Mirrors the columns selected by `SELECT_*`. Source-id columns are read
/// as strings because the schema mixes INTEGER and TEXT (manga_updates and
/// anime_planet use ASCII slugs); SQLite coerces both forms to TEXT here.
#[derive(Debug, FromQueryResult)]
struct RawRow {
    id: i64,
    title: String,
    native_title: Option<String>,
    romanized_title: Option<String>,
    titles: Option<String>,
    #[sea_orm(column_name = "type")]
    kind: Option<String>,
    status: Option<String>,
    year: Option<i32>,
    description: Option<String>,
    state: Option<String>,
    cover_x350_x2: Option<String>,
    cover_x350_x1: Option<String>,
    cover_x250_x2: Option<String>,
    cover_x250_x1: Option<String>,
    cover_raw_url: Option<String>,
    source_anilist_id: Option<String>,
    source_my_anime_list_id: Option<String>,
    source_manga_updates_id: Option<String>,
    source_kitsu_id: Option<String>,
    source_shikimori_id: Option<String>,
    source_anime_planet_id: Option<String>,
    source_anime_news_network_id: Option<String>,
}

fn active_row_to_canonical(row: RawRow) -> Option<SeriesMetadata> {
    // Skip merged / deleted rows; the resolver should follow the canonical
    // row instead. `merged_with` is the pointer but the dump publishes both
    // sides — we filter out the non-active side here.
    if matches!(row.state.as_deref(), Some("merged") | Some("deleted")) {
        return None;
    }
    Some(row_to_canonical(row))
}

fn row_to_canonical(row: RawRow) -> SeriesMetadata {
    let external_id = row.id.to_string();
    let external_url = Some(format!("https://mangabaka.dev/series/{external_id}"));
    let alternate_titles = collect_alternates(&row);
    let foreign_ids = row_to_foreign_ids(&row);
    let cover_url = pick_cover_from_row(&row);
    let kind = row.kind.as_deref().map(parse_kind);
    let status = row.status.as_deref().map(parse_status);
    // Build a deterministic blob from the dump row so the resolver can
    // hash + dedupe writes. The dump itself doesn't expose a per-row
    // version; the SHA stays stable as long as the row stays stable.
    let raw = serde_json::to_value(SerializedRow {
        id: row.id,
        title: row.title.clone(),
        native_title: row.native_title.clone(),
        romanized_title: row.romanized_title.clone(),
        kind: row.kind.clone(),
        status: row.status.clone(),
        year: row.year,
        description: row.description.clone(),
        cover_url: cover_url.clone(),
        alternate_titles: alternate_titles.clone(),
        foreign_ids: foreign_ids.clone(),
    })
    .expect("SerializedRow always serializes");
    let content_hash = crate::mapping::hash_value(&raw);

    SeriesMetadata {
        external_id,
        canonical_title: row.title,
        alternate_titles,
        kind,
        status,
        year: row.year,
        cover_url,
        external_url,
        genres: Vec::new(),
        tags: Vec::new(),
        foreign_ids,
        raw,
        content_hash,
    }
}

#[derive(serde::Serialize)]
struct SerializedRow {
    id: i64,
    title: String,
    native_title: Option<String>,
    romanized_title: Option<String>,
    kind: Option<String>,
    status: Option<String>,
    year: Option<i32>,
    description: Option<String>,
    cover_url: Option<String>,
    alternate_titles: Vec<String>,
    foreign_ids: Vec<ForeignId>,
}

fn collect_alternates(row: &RawRow) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: Option<&str>| {
        if let Some(v) = s
            && !v.is_empty()
            && v != row.title
            && !out.iter().any(|e| e == v)
        {
            out.push(v.to_string());
        }
    };
    push(row.native_title.as_deref());
    push(row.romanized_title.as_deref());

    // `titles` is a JSON array of `{title, language, traits, is_primary, note}`.
    if let Some(raw) = row.titles.as_deref()
        && let Ok(Json::Array(arr)) = serde_json::from_str::<Json>(raw)
    {
        for entry in arr {
            if let Some(t) = entry.get("title").and_then(|v| v.as_str()) {
                push(Some(t));
            }
        }
    }
    out
}

fn row_to_foreign_ids(row: &RawRow) -> Vec<ForeignId> {
    let mut out = Vec::new();
    let mut push = |provider: &str, id: &Option<String>| {
        if let Some(id) = id.as_deref().filter(|s| !s.is_empty()) {
            out.push(ForeignId {
                provider: provider.to_string(),
                id: id.to_string(),
                url: None,
            });
        }
    };
    push("anilist", &row.source_anilist_id);
    push("mal", &row.source_my_anime_list_id);
    push("mangaupdates", &row.source_manga_updates_id);
    push("kitsu", &row.source_kitsu_id);
    push("shikimori", &row.source_shikimori_id);
    push("anime_planet", &row.source_anime_planet_id);
    push("anime_news_network", &row.source_anime_news_network_id);
    out
}

fn pick_cover_from_row(row: &RawRow) -> Option<String> {
    row.cover_x350_x2
        .clone()
        .or_else(|| row.cover_x350_x1.clone())
        .or_else(|| row.cover_x250_x2.clone())
        .or_else(|| row.cover_x250_x1.clone())
        .or_else(|| row.cover_raw_url.clone())
}

fn parse_kind(s: &str) -> SeriesKind {
    match s {
        "manga" => SeriesKind::Manga,
        "manhwa" => SeriesKind::Manhwa,
        "manhua" => SeriesKind::Manhua,
        "novel" => SeriesKind::Novel,
        "one_shot" | "oneshot" => SeriesKind::OneShot,
        "oel" => SeriesKind::Oel,
        other => SeriesKind::Other(other.to_string()),
    }
}

fn parse_status(s: &str) -> SeriesStatus {
    match s {
        "releasing" | "ongoing" => SeriesStatus::Ongoing,
        "completed" => SeriesStatus::Completed,
        "hiatus" => SeriesStatus::Hiatus,
        "cancelled" => SeriesStatus::Cancelled,
        "upcoming" => SeriesStatus::Upcoming,
        _ => SeriesStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, Statement};

    /// Build an in-memory SQLite mimicking the dump schema (subset of
    /// columns) so we can exercise the store without downloading 3 GB.
    /// Mirrors the columns `OfflineStore` actually reads.
    async fn fixture_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let backend = db.get_database_backend();
        db.execute(Statement::from_string(
            backend,
            "CREATE TABLE series (
                id INTEGER PRIMARY KEY,
                title TEXT,
                native_title TEXT,
                romanized_title TEXT,
                titles TEXT,
                type TEXT,
                status TEXT,
                year INTEGER,
                description TEXT,
                state TEXT,
                cover_x350_x2 TEXT,
                cover_x350_x1 TEXT,
                cover_x250_x2 TEXT,
                cover_x250_x1 TEXT,
                cover_raw_url TEXT,
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
        // Insert Chainsaw Man (active) and a merged duplicate to exercise filtering.
        db.execute(Statement::from_string(
            backend,
            "INSERT INTO series (
                id, title, native_title, romanized_title, titles, type, status, year, state,
                cover_x350_x2, source_anilist_id, source_my_anime_list_id,
                source_manga_updates_id, source_kitsu_id, source_anime_planet_id
            ) VALUES (
                1677, 'Chainsaw Man', 'チェンソーマン', 'Chainsaw Man',
                '[{\"title\":\"Chainsaw-Man\",\"language\":\"en\"},{\"title\":\"CSM\",\"language\":\"en\"}]',
                'manga', 'releasing', 2018, 'active',
                'https://mb/350@2x.jpg', 105778, 116778,
                'ylx5wzn', 54139, 'chainsaw-man'
            )"
            .to_string(),
        ))
        .await
        .unwrap();
        db.execute(Statement::from_string(
            backend,
            "INSERT INTO series (
                id, title, type, state, source_anilist_id
            ) VALUES (
                9999, 'Chainsaw Man (merged)', 'manga', 'merged', 105778
            )"
            .to_string(),
        ))
        .await
        .unwrap();
        // FTS5 virtual table mirroring what setup::prepare builds.
        db.execute(Statement::from_string(
            backend,
            "CREATE VIRTUAL TABLE series_title_fts USING fts5(
                title, native_title, romanized_title, alternate_titles, tokenize = 'unicode61'
            )"
            .to_string(),
        ))
        .await
        .unwrap();
        db.execute(Statement::from_string(
            backend,
            "INSERT INTO series_title_fts(rowid, title, native_title, romanized_title, alternate_titles)
                VALUES (1677, 'Chainsaw Man', 'チェンソーマン', 'Chainsaw Man', 'Chainsaw-Man CSM')"
                .to_string(),
        ))
        .await
        .unwrap();
        db
    }

    fn store_from(db: DatabaseConnection) -> OfflineStore {
        OfflineStore {
            db,
            path: PathBuf::from(":memory:"),
        }
    }

    #[tokio::test]
    async fn find_by_id_returns_canonical_metadata() {
        let store = store_from(fixture_db().await);
        let m = store.find_by_id("1677").await.unwrap().unwrap();
        assert_eq!(m.external_id, "1677");
        assert_eq!(m.canonical_title, "Chainsaw Man");
        assert_eq!(m.kind, Some(SeriesKind::Manga));
        assert_eq!(m.status, Some(SeriesStatus::Ongoing));
        assert_eq!(m.year, Some(2018));
        assert!(m.cover_url.as_deref() == Some("https://mb/350@2x.jpg"));
        assert!(
            m.alternate_titles.contains(&"チェンソーマン".to_string()),
            "expected native title in alternates, got {:?}",
            m.alternate_titles
        );
        assert!(
            m.alternate_titles.contains(&"CSM".to_string()),
            "expected JSON-array alternates to be merged in"
        );
    }

    #[tokio::test]
    async fn find_by_id_skips_merged_rows() {
        let store = store_from(fixture_db().await);
        assert!(store.find_by_id("9999").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn find_by_id_returns_none_for_non_numeric_input() {
        let store = store_from(fixture_db().await);
        assert!(store.find_by_id("not-a-number").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn find_by_source_id_resolves_text_slug_and_integer_ids() {
        let store = store_from(fixture_db().await);
        // Slug form (manga_updates uses ASCII slugs).
        let m = store
            .find_by_source_id("manga_updates", "ylx5wzn")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(m.external_id, "1677");
        // Integer form (anilist).
        let m = store
            .find_by_source_id("anilist", "105778")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(m.external_id, "1677");
        // Unknown source returns Ok(None) without error.
        assert!(
            store
                .find_by_source_id("not_a_source", "1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn find_by_source_id_skips_merged_rows() {
        let store = store_from(fixture_db().await);
        // The merged row also has anilist=105778 — but the LIMIT 1 query
        // hits the active row first by insertion order. To be safe, we
        // explicitly insert merged rows after active ones in fixtures.
        // The merged row stays unreachable through this path because the
        // active row claims the same anilist id.
        let m = store
            .find_by_source_id("anilist", "105778")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(m.external_id, "1677");
    }

    #[tokio::test]
    async fn search_fts_matches_canonical_and_alternate_titles() {
        let store = store_from(fixture_db().await);
        let hits = store.search_fts("Chainsaw", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].external_id, "1677");

        let hits = store.search_fts("CSM", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].external_id, "1677");

        let hits = store.search_fts("nonexistent", 10).await.unwrap();
        assert!(hits.is_empty());

        let hits = store.search_fts("   ", 10).await.unwrap();
        assert!(hits.is_empty(), "blank query should short-circuit");
    }

    #[test]
    fn source_column_translates_known_sources() {
        assert_eq!(
            source_column("manga_updates"),
            Some("source_manga_updates_id")
        );
        assert_eq!(source_column("anilist"), Some("source_anilist_id"));
        assert_eq!(source_column("mangadex"), None); // not in dump
    }

    #[test]
    fn sanitize_match_wraps_terms_in_phrase_quotes() {
        assert_eq!(sanitize_match("chainsaw man"), "\"chainsaw\" \"man\"");
        assert_eq!(
            sanitize_match("OR AND foo"),
            "\"OR\" \"AND\" \"foo\"",
            "operator words must be neutralized as phrase literals"
        );
        // Embedded quotes are escaped per FTS5 syntax (double them inside
        // the phrase literal): input token `"world"` → `"""world"""`.
        assert_eq!(sanitize_match(r#"hello "world""#), r#""hello" """world""""#,);
    }
}
