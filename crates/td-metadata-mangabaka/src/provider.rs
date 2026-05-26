//! `MetadataProvider` implementation for MangaBaka.
//!
//! Lookup chain for `get`, `resolve_by_foreign_id`, and `search`:
//!
//! 1. **Negative cache** (in-memory). Recent known-misses short-circuit
//!    without touching the offline store or the network.
//! 2. **Offline cache** (if a [`OfflineStore`] is loaded). Backed by the
//!    extracted MangaBaka dump under
//!    `${storage.provider_cache_dir}/mangabaka/series.sqlite`.
//! 3. **Live API fallback** (if `api_fallback = true`). The HTTP path was
//!    already correct in the API-only build; the offline-first refactor
//!    only adds layers in front of it.
//!
//! `refresh_cache` drives [`crate::offline::refresh`], then re-opens the
//! [`OfflineStore`] under a write lock so concurrent reads see the new
//! data atomically.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use td_config::MangabakaProviderConfig;
use td_metadata::{
    MetadataError, MetadataProvider, MetadataResult, RefreshStatus, RefreshSummary, SearchHit,
    SeriesMetadata,
};
use tokio::sync::RwLock;

use crate::client::MangabakaClient;
use crate::mapping::{series_to_canonical, series_to_search_hit};
use crate::negative_cache::NegativeCache;
use crate::offline::{self, OfflineStore};
use crate::{PROVIDER_DISPLAY_NAME, PROVIDER_ID};

pub struct MangabakaProvider {
    client: MangabakaClient,
    /// Set to true at construction iff `api_key` is configured. Used to
    /// decide whether to fall back to the network on offline misses.
    api_fallback: bool,
    /// Loaded if the on-disk dump exists at construction time, OR after a
    /// successful `refresh_cache`. `None` means "no offline path available
    /// yet"; reads fall straight through to the API (or return Ok(None)).
    offline: RwLock<Option<Arc<OfflineStore>>>,
    /// `${storage.provider_cache_dir}/mangabaka/`. Owned by this provider;
    /// other providers get their own subdirectories.
    cache_dir: PathBuf,
    /// `None` disables refresh entirely (refresh_cache returns NotSupported).
    dump_url: Option<String>,
    negative_cache: NegativeCache,
    http_for_refresh: reqwest::Client,
}

impl MangabakaProvider {
    /// Construct from a typed config block and the provider's owned cache
    /// directory (typically `storage.provider_cache_dir_for("mangabaka")`).
    ///
    /// Errors if `api_fallback = true` is paired with no `api_key`: that
    /// path would surface as a confusing 401 at first request, so reject
    /// loudly at boot.
    pub async fn from_config(
        cfg: &MangabakaProviderConfig,
        cache_dir: PathBuf,
    ) -> Result<Self, MetadataError> {
        if cfg.api_fallback && cfg.api_key.is_none() {
            return Err(MetadataError::NotConfigured {
                provider: PROVIDER_ID.into(),
                message: "api_fallback=true requires providers.mangabaka.api_key; either set the \
                     key or set api_fallback=false (offline-only mode)"
                    .into(),
            });
        }
        let timeout = Duration::from_secs(cfg.timeout_seconds.max(1) as u64);
        let client = MangabakaClient::new(cfg.api_base_url.clone(), cfg.api_key.clone(), timeout)
            .map_err(|e| MetadataError::NotConfigured {
            provider: PROVIDER_ID.into(),
            message: format!("building http client: {e}"),
        })?;

        // Reuse the existing dump if one is on disk; otherwise leave the
        // store unloaded until refresh_cache runs.
        let dump = offline::dump_path(&cache_dir);
        let offline = if dump.exists() {
            match OfflineStore::open_ro(&dump).await {
                Ok(store) => {
                    tracing::info!(path = %dump.display(), "loaded MangaBaka offline cache");
                    Some(Arc::new(store))
                }
                Err(e) => {
                    tracing::warn!(error = ?e, path = %dump.display(), "failed to open existing dump; will retry on next refresh");
                    None
                }
            }
        } else {
            None
        };

        // Use a longer timeout for the refresh client; downloading the
        // dump can take several minutes on a slow link.
        let http_for_refresh = reqwest::Client::builder()
            .user_agent(concat!("tsundoku/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60 * 30))
            .build()
            .map_err(|e| MetadataError::NotConfigured {
                provider: PROVIDER_ID.into(),
                message: format!("building refresh http client: {e}"),
            })?;

        Ok(Self {
            client,
            api_fallback: cfg.api_fallback,
            offline: RwLock::new(offline),
            cache_dir,
            dump_url: cfg
                .offline_dump_url
                .clone()
                .or_else(|| Some(offline::DEFAULT_DUMP_URL.to_string())),
            negative_cache: NegativeCache::new(
                Duration::from_secs(cfg.negative_cache_ttl_days as u64 * 24 * 60 * 60),
                10_000,
            ),
            http_for_refresh,
        })
    }

    /// Snapshot the current offline store under a short-lived read lock so
    /// we don't hold the lock across the .await of the actual query.
    async fn current_offline(&self) -> Option<Arc<OfflineStore>> {
        self.offline.read().await.as_ref().cloned()
    }
}

#[async_trait]
impl MetadataProvider for MangabakaProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn display_name(&self) -> &str {
        PROVIDER_DISPLAY_NAME
    }

    async fn get(&self, external_id: &str) -> MetadataResult<Option<SeriesMetadata>> {
        if self
            .negative_cache
            .is_known_miss(PROVIDER_ID, external_id)
            .await
        {
            return Ok(None);
        }
        if let Some(store) = self.current_offline().await
            && let Some(hit) = store.find_by_id(external_id).await.map_err(map_anyhow)?
        {
            return Ok(Some(hit));
        }
        if !self.api_fallback {
            self.negative_cache
                .record_miss(PROVIDER_ID, external_id)
                .await;
            return Ok(None);
        }
        let api_hit = self
            .client
            .get_series(external_id)
            .await?
            .map(series_to_canonical);
        if api_hit.is_none() {
            self.negative_cache
                .record_miss(PROVIDER_ID, external_id)
                .await;
        }
        Ok(api_hit)
    }

    async fn search(&self, query: &str, limit: u32) -> MetadataResult<Vec<SearchHit>> {
        // Offline FTS only by design: the auto resolver uses local fuzzy
        // matching, and the review UI prefers the offline index when
        // present. API search costs a request per keystroke.
        if let Some(store) = self.current_offline().await {
            return store
                .search_fts(query, limit.max(1))
                .await
                .map_err(map_anyhow);
        }
        // Offline cache not yet loaded; fall back to API search (only path
        // available) so a fresh install still returns hits on the manual
        // review flow.
        if !self.api_fallback {
            return Ok(Vec::new());
        }
        let rows = self.client.search(query, limit.max(1)).await?;
        Ok(rows.iter().map(series_to_search_hit).collect())
    }

    async fn resolve_by_foreign_id(
        &self,
        foreign_provider: &str,
        foreign_id: &str,
    ) -> MetadataResult<Option<SeriesMetadata>> {
        let Some(mb_source) = canonical_to_mb_source(foreign_provider) else {
            return Ok(None);
        };
        let neg_key = format!("{foreign_provider}:{foreign_id}");
        if self
            .negative_cache
            .is_known_miss(PROVIDER_ID, &neg_key)
            .await
        {
            return Ok(None);
        }
        if let Some(store) = self.current_offline().await
            && let Some(hit) = store
                .find_by_source_id(mb_source, foreign_id)
                .await
                .map_err(map_anyhow)?
        {
            return Ok(Some(hit));
        }
        if !self.api_fallback {
            self.negative_cache.record_miss(PROVIDER_ID, &neg_key).await;
            return Ok(None);
        }
        let api_hit = self
            .client
            .get_by_source_id(mb_source, foreign_id)
            .await?
            .map(series_to_canonical);
        if api_hit.is_none() {
            self.negative_cache.record_miss(PROVIDER_ID, &neg_key).await;
        }
        Ok(api_hit)
    }

    async fn offline_cache_loaded(&self) -> bool {
        self.offline.read().await.is_some()
    }

    async fn refresh_cache(&self) -> MetadataResult<RefreshSummary> {
        let Some(dump_url) = self.dump_url.as_deref() else {
            return Ok(RefreshSummary::not_supported(PROVIDER_ID));
        };

        // Close the current store before extracting: Windows can't rename
        // over an open file, and even on POSIX it keeps the reader from
        // seeing torn writes during setup.
        {
            let mut guard = self.offline.write().await;
            *guard = None;
        }

        let mut summary = offline::refresh(
            &self.http_for_refresh,
            dump_url,
            &self.cache_dir,
            Duration::from_secs(60 * 30),
        )
        .await?;

        // Re-open the freshly-written file and count records for the summary.
        let dump = offline::dump_path(&self.cache_dir);
        let store = OfflineStore::open_ro(&dump).await.map_err(map_anyhow)?;
        if let RefreshStatus::Refreshed {
            ref mut records, ..
        } = summary.status
        {
            *records = offline::count_records(&store).await.map_err(map_anyhow)?;
        }
        *self.offline.write().await = Some(Arc::new(store));

        Ok(summary)
    }
}

fn map_anyhow(err: anyhow::Error) -> MetadataError {
    MetadataError::Unavailable {
        provider: PROVIDER_ID.into(),
        source: err,
    }
}

/// Translate our canonical provider id → MangaBaka's `source` URL component.
/// Unknown providers return `None` and are skipped in the resolution chain.
pub(crate) fn canonical_to_mb_source(provider: &str) -> Option<&'static str> {
    match provider {
        "mangaupdates" => Some("manga_updates"),
        "mal" => Some("my_anime_list"),
        "anilist" => Some("anilist"),
        "mangadex" => Some("mangadex"),
        "kitsu" => Some("kitsu"),
        "anime_planet" => Some("anime_planet"),
        "anime_news_network" => Some("anime_news_network"),
        "shikimori" => Some("shikimori"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_api_fallback_without_api_key() {
        let cfg = MangabakaProviderConfig {
            api_key: None,
            api_fallback: true,
            ..Default::default()
        };
        match MangabakaProvider::from_config(&cfg, PathBuf::from("/tmp/td-test-1")).await {
            Err(MetadataError::NotConfigured { provider, .. }) => {
                assert_eq!(provider, "mangabaka");
            }
            Err(other) => panic!("expected NotConfigured, got {other:?}"),
            Ok(_) => panic!("expected from_config to reject api_fallback without api_key"),
        }
    }

    #[tokio::test]
    async fn accepts_offline_only_without_api_key() {
        let cfg = MangabakaProviderConfig {
            api_key: None,
            api_fallback: false,
            ..Default::default()
        };
        let provider = MangabakaProvider::from_config(&cfg, PathBuf::from("/tmp/td-test-2"))
            .await
            .unwrap();
        assert_eq!(provider.id(), "mangabaka");
    }

    #[tokio::test]
    async fn refresh_cache_returns_not_supported_when_dump_url_is_none() {
        let cfg = MangabakaProviderConfig {
            offline_dump_url: Some(String::new()), // overridden below
            ..Default::default()
        };
        let provider = MangabakaProvider::from_config(&cfg, PathBuf::from("/tmp/td-test-3"))
            .await
            .unwrap();
        // Force-disable the dump url to exercise the NotSupported branch.
        // (We can't reach private fields directly; mutate a fresh instance
        // by recreating with `offline_dump_url = None`. The default impl
        // synthesizes a url, so we route around it here.)
        let mut clean = provider;
        clean.dump_url = None;
        let s = clean.refresh_cache().await.unwrap();
        assert!(matches!(s.status, RefreshStatus::NotSupported));
    }

    #[tokio::test]
    async fn offline_cache_loaded_reports_false_when_no_dump_on_disk() {
        let cfg = MangabakaProviderConfig {
            api_key: None,
            api_fallback: false,
            ..Default::default()
        };
        let provider =
            MangabakaProvider::from_config(&cfg, PathBuf::from("/tmp/td-test-offline-loaded"))
                .await
                .unwrap();
        assert!(!provider.offline_cache_loaded().await);
    }

    #[test]
    fn canonical_to_mb_source_round_trip() {
        assert_eq!(
            canonical_to_mb_source("mangaupdates"),
            Some("manga_updates")
        );
        assert_eq!(canonical_to_mb_source("mal"), Some("my_anime_list"));
        assert_eq!(canonical_to_mb_source("anilist"), Some("anilist"));
        assert_eq!(canonical_to_mb_source("unknown_provider"), None);
    }
}
