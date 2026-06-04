//! Sanity-check an extracted MangaBaka dump before it is promoted into the
//! live cache slot.
//!
//! The integrity guarantee used to rest on the published `.sha1` sidecar,
//! but upstream lets that sidecar drift out of sync with the tarball it
//! serves (the dump gets republished without the sidecar catching up), so a
//! hash mismatch can no longer be read as corruption. Instead we validate
//! the *content*: open the extracted file read-only and require a
//! well-formed `series` table carrying a plausible row count. A truncated or
//! partially written download either fails to open or falls under the floor.
//!
//! This is a fast pre-flight; it runs *before* the multi-minute index build
//! in [`super::setup::prepare`] so a bad download fails immediately instead
//! of after minutes of wasted work. It does not attempt a full
//! `PRAGMA integrity_check` — that reads every page and would add minutes to
//! every refresh; gross corruption surfaces here, and `prepare` reads the
//! whole `series` table for the FTS build right after.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use sea_orm::{ConnectionTrait, Database, Statement};

/// Floor on the `series` row count an extracted dump must carry to be
/// considered intact. The published dump holds ~585k rows; anything far
/// below this is a truncated or corrupt extract, not a legitimately small
/// dump. Deliberately conservative (well under the real count) so a future
/// upstream prune doesn't trip a false alarm.
pub const MIN_SERIES_ROWS: i64 = 100_000;

/// Validate the extracted dump at `path`, requiring at least
/// [`MIN_SERIES_ROWS`] rows in `series`.
pub async fn validate_dump(path: impl AsRef<Path>) -> Result<()> {
    validate_with_floor(path, MIN_SERIES_ROWS).await
}

/// Floor-parameterised core, split out so tests can assert the boundary
/// without materialising 100k rows.
async fn validate_with_floor(path: impl AsRef<Path>, min_rows: i64) -> Result<()> {
    let path = path.as_ref();
    if !path.exists() {
        bail!("extracted dump not found at {}", path.display());
    }
    let url = format!("sqlite://{}?mode=ro", path.display());
    let db = Database::connect(&url)
        .await
        .with_context(|| format!("opening extracted dump for validation {}", path.display()))?;
    let backend = db.get_database_backend();

    // A corrupt header or a missing `series` table surfaces as a query error
    // here, which is exactly the signal we want.
    let row = db
        .query_one(Statement::from_string(
            backend,
            "SELECT COUNT(*) AS n FROM series".to_string(),
        ))
        .await
        .context("counting series rows in extracted dump (table missing or file corrupt?)")?
        .ok_or_else(|| anyhow!("COUNT(*) returned no row"))?;
    let n: i64 = row.try_get("", "n").context("reading series count")?;

    if n < min_rows {
        bail!(
            "extracted dump has only {n} series rows (expected >= {min_rows}); \
             treating as a corrupt or truncated download"
        );
    }

    tracing::info!(series_rows = n, "extracted dump validated");
    drop(db);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, Statement};
    use tempfile::TempDir;

    async fn make_series_db(dir: &TempDir, rows: i64) -> std::path::PathBuf {
        let path = dir.path().join("series.sqlite");
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let db = Database::connect(&url).await.unwrap();
        let backend = db.get_database_backend();
        db.execute(Statement::from_string(
            backend,
            "CREATE TABLE series (id INTEGER PRIMARY KEY, title TEXT)".to_string(),
        ))
        .await
        .unwrap();
        for i in 1..=rows {
            db.execute(Statement::from_string(
                backend,
                format!("INSERT INTO series (id, title) VALUES ({i}, 'row {i}')"),
            ))
            .await
            .unwrap();
        }
        drop(db);
        path
    }

    #[tokio::test]
    async fn passes_when_row_count_meets_floor() {
        let dir = TempDir::new().unwrap();
        let path = make_series_db(&dir, 3).await;
        validate_with_floor(&path, 2).await.unwrap();
    }

    #[tokio::test]
    async fn fails_when_row_count_below_floor() {
        let dir = TempDir::new().unwrap();
        let path = make_series_db(&dir, 3).await;
        let err = validate_with_floor(&path, 100).await.unwrap_err();
        assert!(
            err.to_string().contains("only 3 series rows"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn fails_when_series_table_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("series.sqlite");
        // A valid SQLite file with no `series` table.
        let db = Database::connect(format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .unwrap();
        db.execute(Statement::from_string(
            db.get_database_backend(),
            "CREATE TABLE other (id INTEGER PRIMARY KEY)".to_string(),
        ))
        .await
        .unwrap();
        drop(db);

        assert!(validate_with_floor(&path, 1).await.is_err());
    }

    #[tokio::test]
    async fn fails_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nope.sqlite");
        assert!(validate_with_floor(&path, 1).await.is_err());
    }
}
