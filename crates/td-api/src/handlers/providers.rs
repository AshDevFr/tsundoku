//! Metadata-provider listing + manual cache-refresh trigger + ad-hoc
//! provider search (used by the review-queue link modal).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use td_config::{MangabakaProviderConfig, ProvidersConfig};
use td_db::repos::provider_cache_state_repo;
use td_metadata::{MetadataProvider, SeriesKind, SeriesStatus};
use td_resolution::scoring::dice;
use td_scheduler::jobs::refresh_provider_cache;
use utoipa::{IntoParams, ToSchema};

use crate::errors::{ApiError, ApiResult};
use crate::state::{AppState, JobEvent, JobKind, JobResult};

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCacheState {
    pub fetched_at: i64,
    pub cache_version: Option<String>,
    pub record_count: Option<i64>,
    pub bytes_downloaded: Option<i64>,
    pub source_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDto {
    pub id: String,
    pub display_name: String,
    pub active: bool,
    pub last_refresh: Option<ProviderCacheState>,
    /// Operator-facing snapshot of the `[providers.<id>]` config block. May
    /// be `None` for providers that don't have a typed config block (today
    /// this only happens for test doubles).
    pub config: Option<ProviderConfigDto>,
}

/// Per-provider config exposed to the admin UI. The api_key is reported as
/// a boolean only; the raw value never leaves the process.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfigDto {
    pub api_fallback: bool,
    /// `true` when an api_key is configured (any non-empty value). The
    /// actual key never appears in JSON; that's the whole point of stashing
    /// it in `.env`.
    pub api_key_set: bool,
    pub api_base_url: String,
    pub offline_dump_url: Option<String>,
    pub offline_dump_configured: bool,
    /// Runtime: whether the on-disk dump is currently loaded.
    pub offline_cache_loaded: bool,
    pub offline_refresh_cron: Option<String>,
    pub negative_cache_ttl_days: u32,
    pub timeout_seconds: u32,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderList {
    pub items: Vec<ProviderDto>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResponse {
    pub provider: String,
    pub triggered: bool,
    /// `false` when a refresh is already in flight; the request is a no-op.
    pub skipped: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefreshAllResponse {
    pub results: Vec<RefreshResponse>,
}

fn mangabaka_config_dto(
    cfg: &MangabakaProviderConfig,
    offline_cache_loaded: bool,
) -> ProviderConfigDto {
    let api_key_set = cfg
        .api_key
        .as_deref()
        .map(|k| !k.is_empty())
        .unwrap_or(false);
    let offline_dump_configured = cfg
        .offline_dump_url
        .as_deref()
        .map(|u| !u.is_empty())
        .unwrap_or(false);
    ProviderConfigDto {
        api_fallback: cfg.api_fallback,
        api_key_set,
        api_base_url: cfg.api_base_url.clone(),
        offline_dump_url: cfg.offline_dump_url.clone(),
        offline_dump_configured,
        offline_cache_loaded,
        offline_refresh_cron: cfg.offline_refresh_cron.clone(),
        negative_cache_ttl_days: cfg.negative_cache_ttl_days,
        timeout_seconds: cfg.timeout_seconds,
    }
}

async fn build_provider_config_dto(
    provider_id: &str,
    provider: &Arc<dyn MetadataProvider>,
    providers_cfg: &ProvidersConfig,
) -> Option<ProviderConfigDto> {
    match provider_id {
        "mangabaka" => Some(mangabaka_config_dto(
            &providers_cfg.mangabaka,
            provider.offline_cache_loaded().await,
        )),
        _ => None,
    }
}

/// List every registered metadata provider with its latest cache-refresh
/// markers and the active-provider flag.
#[utoipa::path(
    get,
    path = "/api/v1/providers",
    tag = "providers",
    operation_id = "list_providers",
    responses((status = 200, body = ProviderList))
)]
pub async fn list(State(state): State<AppState>) -> ApiResult<Json<ProviderList>> {
    let active = state.metadata.active_id().to_string();
    let mut items = Vec::with_capacity(state.metadata.len());
    for (id, provider) in state.metadata.iter() {
        let latest = provider_cache_state_repo::latest(&state.db, id)
            .await
            .map_err(ApiError::Internal)?;
        let config = build_provider_config_dto(id, provider, &state.providers_config).await;
        items.push(ProviderDto {
            id: id.to_string(),
            display_name: provider.display_name().to_string(),
            active: id == active,
            last_refresh: latest.map(|r| ProviderCacheState {
                fetched_at: r.fetched_at,
                cache_version: r.cache_version,
                record_count: r.record_count,
                bytes_downloaded: r.bytes_downloaded,
                source_url: r.source_url,
            }),
            config,
        });
    }
    items.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Json(ProviderList { items }))
}

/// Trigger an offline-cache refresh for the named provider. Same locking
/// semantics as the scheduled refresh job; an in-flight refresh causes
/// this request to report `skipped = true` without spawning a duplicate.
#[utoipa::path(
    post,
    path = "/api/v1/providers/{id}/refresh-cache",
    tag = "providers",
    params(("id" = String, Path, description = "Provider id")),
    responses(
        (status = 202, body = RefreshResponse),
        (status = 404, description = "No provider with that id registered")
    ),
    security(("admin" = []))
)]
pub async fn refresh_cache(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<RefreshResponse>> {
    let provider = state
        .metadata
        .get(&id)
        .cloned()
        .ok_or_else(|| ApiError::NotFound(format!("provider {id:?}")))?;

    let lock = state.locks.provider_lock(&id);
    let skipped = lock.try_lock().is_err();
    if skipped {
        state.send_job_event(JobEvent::finished(
            JobKind::Provider,
            &id,
            JobResult {
                triggered: false,
                skipped: true,
                ..Default::default()
            },
        ));
        return Ok(Json(RefreshResponse {
            provider: id,
            triggered: false,
            skipped: true,
        }));
    }
    // Drop the test-lock; the spawned tick re-acquires it. Same race
    // tolerance as the sources handler.

    state.send_job_event(JobEvent::started(JobKind::Provider, &id));

    let db = state.db.clone();
    let locks = state.locks.clone();
    let events = state.job_events.clone();
    let event_id = id.clone();
    tokio::spawn(async move {
        refresh_provider_cache::run_tick(provider, db, locks, "manual").await;
        let _ = events.send(JobEvent::finished(
            JobKind::Provider,
            event_id,
            JobResult {
                triggered: true,
                skipped: false,
                ..Default::default()
            },
        ));
    });

    Ok(Json(RefreshResponse {
        provider: id,
        triggered: true,
        skipped: false,
    }))
}

/// Fan-out cache refresh for every registered provider. Returns a per-id
/// triggered/skipped breakdown so the admin UI can render each result.
#[utoipa::path(
    post,
    path = "/api/v1/providers/refresh-all",
    tag = "providers",
    responses((status = 202, body = RefreshAllResponse)),
    security(("admin" = []))
)]
pub async fn refresh_all(State(state): State<AppState>) -> ApiResult<Json<RefreshAllResponse>> {
    let mut ids: Vec<String> = state
        .metadata
        .iter()
        .map(|(id, _)| id.to_string())
        .collect();
    ids.sort();

    let mut results = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(provider) = state.metadata.get(&id).cloned() else {
            continue;
        };
        let lock = state.locks.provider_lock(&id);
        if lock.try_lock().is_err() {
            state.send_job_event(JobEvent::finished(
                JobKind::Provider,
                &id,
                JobResult {
                    triggered: false,
                    skipped: true,
                    ..Default::default()
                },
            ));
            results.push(RefreshResponse {
                provider: id,
                triggered: false,
                skipped: true,
            });
            continue;
        }
        state.send_job_event(JobEvent::started(JobKind::Provider, &id));
        let db = state.db.clone();
        let locks = state.locks.clone();
        let events = state.job_events.clone();
        let event_id = id.clone();
        tokio::spawn(async move {
            refresh_provider_cache::run_tick(provider, db, locks, "manual").await;
            let _ = events.send(JobEvent::finished(
                JobKind::Provider,
                event_id,
                JobResult {
                    triggered: true,
                    skipped: false,
                    ..Default::default()
                },
            ));
        });
        results.push(RefreshResponse {
            provider: id,
            triggered: true,
            skipped: false,
        });
    }

    Ok(Json(RefreshAllResponse { results }))
}

/// Query string for [`search`]. Exactly one of `q` / `external_id` is
/// required — when both are present, `external_id` wins (lookup is
/// faster and more precise than search).
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSearchQuery {
    /// Free-text title query. Trimmed before use; ignored when empty.
    #[serde(default)]
    pub q: Option<String>,
    /// Direct provider external-id lookup. When set, the handler short-
    /// circuits to `MetadataProvider::get` and returns at most one hit
    /// with `score = 1.0`.
    #[serde(default)]
    pub external_id: Option<String>,
    /// Maximum number of hits to enrich with full metadata. Defaults to
    /// 10; clamped to `[1, 50]` to keep the per-request cost bounded.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// One enriched hit. `score` is Dice(`q`, `title`) for the title-search
/// path, or `1.0` for the externalId short-circuit path.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSearchHit {
    pub external_id: String,
    pub title: String,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    /// First alternate title, if any. Useful for showing the
    /// romaji/Japanese form alongside the canonical English title.
    pub native_title: Option<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub external_url: Option<String>,
    pub score: f32,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSearchResponse {
    pub provider: String,
    pub hits: Vec<ProviderSearchHit>,
}

/// Title-or-externalId search against a single provider. Powers the
/// review-queue "Link release" modal.
///
/// - `?externalId=<id>` — direct lookup via `MetadataProvider::get`.
/// - `?q=<title>` — `MetadataProvider::search`, then enrichment via
///   `get` on the top N hits and Dice-rescoring against `q`.
/// - both empty → `400 Bad Request`.
/// - unknown `id` → `404 Not Found`.
#[utoipa::path(
    get,
    path = "/api/v1/providers/{id}/search",
    tag = "providers",
    operation_id = "search_provider",
    params(
        ("id" = String, Path, description = "Provider id"),
        ProviderSearchQuery,
    ),
    responses(
        (status = 200, body = ProviderSearchResponse),
        (status = 400, description = "Both q and externalId missing or empty"),
        (status = 404, description = "Unknown provider id")
    )
)]
pub async fn search(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<ProviderSearchQuery>,
) -> ApiResult<Json<ProviderSearchResponse>> {
    let provider = state
        .metadata
        .get(&id)
        .cloned()
        .ok_or_else(|| ApiError::NotFound(format!("provider {id:?}")))?;

    let external_id = params
        .external_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let q = params.q.as_deref().map(str::trim).filter(|s| !s.is_empty());

    // externalId path: precise lookup, single hit at score 1.0.
    if let Some(external_id) = external_id {
        let meta = provider
            .get(external_id)
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("provider get failed: {e}")))?;
        let hits = match meta {
            Some(m) => vec![enrich(m, 1.0)],
            None => Vec::new(),
        };
        return Ok(Json(ProviderSearchResponse { provider: id, hits }));
    }

    // Title path: search → enrich top N → Dice-rescore.
    let q =
        q.ok_or_else(|| ApiError::BadRequest("either q or externalId is required".to_string()))?;
    let limit = params.limit.unwrap_or(25).clamp(1, 100);
    let hits = provider
        .search(q, limit)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("provider search failed: {e}")))?;

    let mut enriched = Vec::with_capacity(hits.len());
    for hit in hits {
        let score = dice(q, &hit.title);
        // Pull full metadata so the UI gets cover/kind/tags. Skip on
        // miss or error (the search returned an ID we then couldn't
        // resolve — surface the stub anyway with empty enrichment).
        match provider.get(&hit.external_id).await {
            Ok(Some(m)) => enriched.push(enrich(m, score)),
            Ok(None) => enriched.push(stub_hit(hit, score)),
            Err(e) => {
                tracing::warn!(error = ?e, provider = %id, external_id = %hit.external_id,
                    "provider get failed during search enrichment; emitting stub");
                enriched.push(stub_hit(hit, score));
            }
        }
    }
    enriched.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(Json(ProviderSearchResponse {
        provider: id,
        hits: enriched,
    }))
}

fn enrich(m: td_metadata::SeriesMetadata, score: f32) -> ProviderSearchHit {
    ProviderSearchHit {
        external_id: m.external_id,
        title: m.canonical_title,
        year: m.year,
        cover_url: m.cover_url,
        kind: m.kind.as_ref().map(series_kind_str),
        status: m.status.as_ref().map(series_status_str),
        native_title: m.alternate_titles.into_iter().next(),
        genres: m.genres,
        tags: m.tags,
        external_url: m.external_url,
        score,
    }
}

fn stub_hit(hit: td_metadata::SearchHit, score: f32) -> ProviderSearchHit {
    ProviderSearchHit {
        external_id: hit.external_id,
        title: hit.title,
        year: hit.year,
        cover_url: hit.cover_url,
        kind: None,
        status: None,
        native_title: None,
        genres: Vec::new(),
        tags: Vec::new(),
        external_url: None,
        score,
    }
}

fn series_kind_str(k: &SeriesKind) -> String {
    match k {
        SeriesKind::Manga => "manga".into(),
        SeriesKind::Manhwa => "manhwa".into(),
        SeriesKind::Manhua => "manhua".into(),
        SeriesKind::Novel => "novel".into(),
        SeriesKind::OneShot => "one_shot".into(),
        SeriesKind::Oel => "oel".into(),
        SeriesKind::Other(s) => s.clone(),
    }
}

fn series_status_str(s: &SeriesStatus) -> String {
    match s {
        SeriesStatus::Ongoing => "ongoing".into(),
        SeriesStatus::Completed => "completed".into(),
        SeriesStatus::Hiatus => "hiatus".into(),
        SeriesStatus::Cancelled => "cancelled".into(),
        SeriesStatus::Upcoming => "upcoming".into(),
        SeriesStatus::Unknown => "unknown".into(),
    }
}
