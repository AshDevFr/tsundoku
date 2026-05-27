//! Real-dump smoke test. Only runs if the env var
//! `TSUNDOKU_MB_DUMP_FIXTURE` points at an extracted `series.sqlite` file.
//! Use it to exercise the offline-cache pipeline against the actual
//! MangaBaka dataset without re-downloading on every test run:
//!
//! ```bash
//! curl -sL -o /tmp/mb.tar.gz https://api.mangabaka.dev/v1/database/series.sqlite.tar.gz
//! tar -xzf /tmp/mb.tar.gz -C /tmp/mb-inspect
//! TSUNDOKU_MB_DUMP_FIXTURE=/tmp/mb-inspect/series.sqlite cargo test -p td-metadata-mangabaka --test real_dump -- --nocapture
//! ```
//!
//! The test copies the fixture into a temp dir (the file is mutated by
//! `setup::prepare` to add indexes + FTS), so the source fixture is left
//! untouched.

use std::path::PathBuf;

use td_metadata::MetadataProvider;
use td_metadata_mangabaka::{OfflineStore, offline::setup};

fn fixture_path() -> Option<PathBuf> {
    std::env::var("TSUNDOKU_MB_DUMP_FIXTURE")
        .ok()
        .map(PathBuf::from)
}

async fn prepared_dump_in(dir: &tempfile::TempDir) -> Option<PathBuf> {
    let src = fixture_path()?;
    if !src.exists() {
        eprintln!("fixture {} does not exist; skipping", src.display());
        return None;
    }
    let dst = dir.path().join("series.sqlite");
    std::fs::copy(&src, &dst).expect("copy fixture into temp dir");
    setup::prepare(&dst).await.expect("setup::prepare");
    Some(dst)
}

#[tokio::test]
async fn offline_store_finds_known_series_by_id() {
    let dir = tempfile::TempDir::new().unwrap();
    let Some(path) = prepared_dump_in(&dir).await else {
        return;
    };
    let store = OfflineStore::open_ro(&path).await.unwrap();
    let chainsaw = store.find_by_id("1677").await.unwrap();
    let chainsaw = chainsaw.expect("series id 1677 (Chainsaw Man) must exist in dump");
    assert_eq!(chainsaw.canonical_title, "Chainsaw Man");
    assert!(
        !chainsaw.foreign_ids.is_empty(),
        "Chainsaw Man should surface cross-provider IDs"
    );
}

#[tokio::test]
async fn offline_store_resolves_real_foreign_ids() {
    let dir = tempfile::TempDir::new().unwrap();
    let Some(path) = prepared_dump_in(&dir).await else {
        return;
    };
    let store = OfflineStore::open_ro(&path).await.unwrap();

    // Chainsaw Man's AniList ID is 105778 (integer column).
    let hit = store
        .find_by_source_id("anilist", "105778")
        .await
        .unwrap()
        .expect("anilist 105778 should resolve to Chainsaw Man");
    assert_eq!(hit.canonical_title, "Chainsaw Man");

    // Chainsaw Man's MangaUpdates ID is "ylx5wzn" (text/slug column).
    let hit = store
        .find_by_source_id("manga_updates", "ylx5wzn")
        .await
        .unwrap()
        .expect("manga_updates ylx5wzn should resolve to Chainsaw Man");
    assert_eq!(hit.canonical_title, "Chainsaw Man");
}

#[tokio::test]
async fn offline_store_fts_matches_a_well_known_title() {
    let dir = tempfile::TempDir::new().unwrap();
    let Some(path) = prepared_dump_in(&dir).await else {
        return;
    };
    let store = OfflineStore::open_ro(&path).await.unwrap();
    let hits = store.search_fts("Chainsaw Man", 5).await.unwrap();
    assert!(
        !hits.is_empty(),
        "FTS should return at least one hit for 'Chainsaw Man'"
    );
    assert!(
        hits.iter().any(|h| h.title.contains("Chainsaw")),
        "expected a 'Chainsaw' title in top FTS hits, got {hits:?}"
    );
}

#[tokio::test]
async fn provider_loads_existing_dump_at_startup() {
    // Constructing a MangabakaProvider with a pre-extracted dump in its
    // cache_dir should result in `get` resolving against the offline store
    // without any network call.
    let dir = tempfile::TempDir::new().unwrap();
    let Some(_path) = prepared_dump_in(&dir).await else {
        return;
    };
    // The provider expects the file at `${cache_dir}/series.sqlite`, which
    // is exactly where `prepared_dump_in` writes it.
    let cfg = td_config::MangabakaProviderConfig {
        api_fallback: false, // force offline-only so any miss returns Ok(None)
        ..Default::default()
    };
    let provider = td_metadata_mangabaka::MangabakaProvider::from_config(
        &cfg,
        dir.path().to_path_buf(),
        td_http::HttpLimiter::no_limit(),
    )
    .await
    .unwrap();
    let hit = provider.get("1677").await.unwrap();
    let hit = hit.expect("provider.get(1677) should hit the offline store");
    assert_eq!(hit.canonical_title, "Chainsaw Man");

    // Foreign-id resolution short-circuits in the offline store.
    let cross = provider
        .resolve_by_foreign_id("mangaupdates", "ylx5wzn")
        .await
        .unwrap();
    let cross = cross.expect("mangaupdates ylx5wzn should resolve offline");
    assert_eq!(cross.canonical_title, "Chainsaw Man");
}

#[tokio::test]
async fn provider_returns_none_for_missing_id_without_api() {
    let dir = tempfile::TempDir::new().unwrap();
    let Some(_path) = prepared_dump_in(&dir).await else {
        return;
    };
    let cfg = td_config::MangabakaProviderConfig {
        api_fallback: false,
        ..Default::default()
    };
    let provider = td_metadata_mangabaka::MangabakaProvider::from_config(
        &cfg,
        dir.path().to_path_buf(),
        td_http::HttpLimiter::no_limit(),
    )
    .await
    .unwrap();
    let hit = provider.get("999999999").await.unwrap();
    assert!(
        hit.is_none(),
        "non-existent id should return None in offline-only mode"
    );
}
