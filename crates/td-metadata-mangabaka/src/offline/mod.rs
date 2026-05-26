//! MangaBaka offline-cache subsystem.
//!
//! Pipeline:
//!
//! 1. [`fetch::download`] streams `series.sqlite.tar.gz` from MangaBaka,
//!    optionally cross-checking the `.sha1` sidecar.
//! 2. [`extract::extract_dump`] untars the archive into a temp `series.sqlite`.
//! 3. [`setup::prepare`] adds source-id indexes + an FTS5 virtual table on
//!    the extracted file (a one-time cost per refresh, ~minutes on 585k rows).
//! 4. [`store::OfflineStore::open_ro`] opens the prepared file as a
//!    read-only sea-orm connection for the provider to query.
//!
//! `refresh_cache()` glues these together with atomic file swaps so an
//! in-flight request never sees a half-written DB.

pub mod extract;
pub mod fetch;
pub mod setup;
pub mod store;

pub use store::OfflineStore;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, anyhow};
use chrono::Utc;
use td_metadata::{MetadataError, RefreshStatus, RefreshSummary};

/// Canonical filename for the extracted dump under
/// `${provider_cache_dir}/mangabaka/`.
pub const DUMP_FILENAME: &str = "series.sqlite";

/// Subdirectory used for in-progress downloads and extractions. Kept inside
/// the cache dir so the temp lives on the same filesystem as the target
/// (cheap atomic rename).
pub const TMP_SUBDIR: &str = ".tmp";

/// Default URL for the SQLite dump tarball.
pub const DEFAULT_DUMP_URL: &str = "https://api.mangabaka.dev/v1/database/series.sqlite.tar.gz";

/// Path the live OfflineStore opens from.
pub fn dump_path(cache_dir: impl AsRef<Path>) -> PathBuf {
    cache_dir.as_ref().join(DUMP_FILENAME)
}

/// Path used for in-progress work. Cleared on success.
pub fn tmp_dir(cache_dir: impl AsRef<Path>) -> PathBuf {
    cache_dir.as_ref().join(TMP_SUBDIR)
}

/// Refresh the offline cache. Streams the tarball, optionally verifies its
/// SHA-1 against the published sidecar, extracts `series.sqlite`, adds
/// indexes + FTS5, and atomically renames the prepared file into place.
///
/// Returns a [`RefreshSummary`] for the CLI / API to render. The caller is
/// responsible for closing the previous [`OfflineStore`] before invoking
/// this — extraction writes to a temp path, but the final rename will fail
/// on Windows (and confuse macOS) if the destination is open.
pub async fn refresh(
    http: &reqwest::Client,
    dump_url: &str,
    cache_dir: impl AsRef<Path>,
    timeout: Duration,
) -> Result<RefreshSummary, MetadataError> {
    let started_at = Utc::now();
    let summary = match refresh_inner(http, dump_url, cache_dir.as_ref(), timeout).await {
        Ok(s) => s,
        Err(e) => {
            return Err(MetadataError::Unavailable {
                provider: crate::PROVIDER_ID.into(),
                source: e,
            });
        }
    };
    Ok(RefreshSummary {
        provider: crate::PROVIDER_ID.into(),
        status: summary.status,
        started_at,
        finished_at: Utc::now(),
        bytes_downloaded: Some(summary.bytes_downloaded),
    })
}

struct InnerSummary {
    status: RefreshStatus,
    bytes_downloaded: u64,
}

async fn refresh_inner(
    http: &reqwest::Client,
    dump_url: &str,
    cache_dir: &Path,
    _timeout: Duration,
) -> anyhow::Result<InnerSummary> {
    tokio::fs::create_dir_all(cache_dir)
        .await
        .with_context(|| format!("creating {}", cache_dir.display()))?;
    let tmp = tmp_dir(cache_dir);
    tokio::fs::create_dir_all(&tmp)
        .await
        .with_context(|| format!("creating {}", tmp.display()))?;
    let archive_path = tmp.join("series.sqlite.tar.gz");
    let extracted_path = tmp.join(DUMP_FILENAME);
    let dest_path = dump_path(cache_dir);

    // Download. Best-effort SHA-1 verification against the sidecar; we
    // record a warning if the sidecar is unreachable but do not abort
    // (some mirrors don't host the sidecar).
    tracing::info!(%dump_url, "downloading MangaBaka dump");
    let download = fetch::download(http, dump_url, &archive_path).await?;
    let sidecar_url = fetch::sha1_sidecar_url(dump_url);
    match fetch::fetch_expected_sha1(http, &sidecar_url).await {
        Ok(expected) if expected != download.sha1_hex => {
            return Err(anyhow!(
                "SHA-1 mismatch for {dump_url}: expected {expected}, got {}",
                download.sha1_hex
            ));
        }
        Ok(_) => tracing::info!("SHA-1 sidecar matched"),
        Err(e) => {
            tracing::warn!(error = ?e, sidecar = %sidecar_url, "sha1 sidecar unavailable; proceeding without verification");
        }
    }

    // Extract.
    tracing::info!(archive = %archive_path.display(), "extracting dump");
    extract::extract_dump(&archive_path, &extracted_path).await?;

    // Setup (indexes + FTS).
    setup::prepare(&extracted_path).await?;

    // Atomic swap into place. On most filesystems `rename` over an existing
    // file is atomic; we defensively remove first on Windows where it isn't.
    if dest_path.exists() {
        let _ = tokio::fs::remove_file(&dest_path).await;
    }
    tokio::fs::rename(&extracted_path, &dest_path)
        .await
        .with_context(|| {
            format!(
                "renaming {} -> {}",
                extracted_path.display(),
                dest_path.display()
            )
        })?;

    // Cleanup the tarball. Leave the tmp dir in place; it's tiny.
    let _ = tokio::fs::remove_file(&archive_path).await;

    let bytes = download.bytes_downloaded;
    Ok(InnerSummary {
        status: RefreshStatus::Refreshed {
            records: 0, // populated by the caller when it re-opens the store
            version: Some(download.sha1_hex),
        },
        bytes_downloaded: bytes,
    })
}

/// After a successful refresh, count the active series rows so the
/// `RefreshSummary` reports an accurate record count.
pub async fn count_records(store: &OfflineStore) -> anyhow::Result<u64> {
    use sea_orm::{ConnectionTrait, Statement};
    // The trait API doesn't expose the connection; we instead query through
    // a fresh RO open. Cheap for SQLite.
    let url = format!("sqlite://{}?mode=ro", store.path().display());
    let db = sea_orm::Database::connect(&url).await?;
    let backend = db.get_database_backend();
    let row = db
        .query_one(Statement::from_string(
            backend,
            "SELECT COUNT(*) AS n FROM series WHERE state = 'active' OR state IS NULL".to_string(),
        ))
        .await?
        .ok_or_else(|| anyhow!("COUNT(*) returned no row"))?;
    let n: i64 = row.try_get("", "n")?;
    Ok(n.max(0) as u64)
}
