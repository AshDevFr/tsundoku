//! Aggregate metrics endpoints for the admin metrics UI.
//!
//! Two pipelines mirror each other on purpose:
//!
//! - Sources: per-tick history of poll attempts.
//! - Providers: per-tick history of cache refreshes.
//!
//! For each:
//!
//! - `GET /metrics/sources` returns a per-source summary (success rate,
//!   counts, last run) over a sliding window.
//! - `GET /metrics/sources/{name}` returns the same summary plus per-bucket
//!   counts so the frontend can render a sparkline without re-bucketing on
//!   the client.
//!
//! `range` is `<n>[smhdw]` (e.g. `24h`, `7d`). `buckets` defaults to 24 and
//! is clamped to the safe range [4, 168].

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use td_db::repos::{
    mangaupdates_id_repo, review_snapshots_repo, run_metrics_repo, series_external_ids_repo,
};
use utoipa::{IntoParams, ToSchema};

use crate::errors::{ApiError, ApiResult};
use crate::state::AppState;

const DEFAULT_BUCKETS: u32 = 24;
const MIN_BUCKETS: u32 = 4;
const MAX_BUCKETS: u32 = 168;
const DEFAULT_RANGE_SECONDS: i64 = 24 * 60 * 60;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(default, rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct MetricsQuery {
    /// Sliding window for the aggregate, e.g. `24h`, `7d`. Defaults to 24h.
    pub range: Option<String>,
    /// Number of equal-width buckets. Clamped to [4, 168]. Defaults to 24.
    pub buckets: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionOutcomeBreakdown {
    pub known_id: i64,
    pub foreign_id: i64,
    pub fuzzy: i64,
    pub review: i64,
    pub failed: i64,
}

#[derive(Debug, Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct FetchLatencyDto {
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub max_ms: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct TimeToResolutionDto {
    pub p50_seconds: Option<f64>,
    pub p95_seconds: Option<f64>,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorKindBucket {
    /// `None` is stored when a failure row pre-dates the error_kind helper
    /// or comes from an unwrapped legacy path; surface it as `unknown`.
    pub kind: String,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceMetricsSummaryItem {
    pub source_name: String,
    pub total_runs: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub skipped_count: i64,
    pub fetched_sum: Option<i64>,
    pub new_sum: Option<i64>,
    pub resolved_sum: Option<i64>,
    /// Total milliseconds spent in `DiscoverySource::enrich()` across every
    /// run in the window. `null` when no runs reported it.
    pub enrich_duration_ms_sum: Option<i64>,
    /// Total milliseconds spent in `Resolver::resolve_one()` across every
    /// run in the window. `null` when no runs reported it.
    pub resolve_duration_ms_sum: Option<i64>,
    pub last_started_at: Option<i64>,
    pub last_status: Option<String>,
    /// Convenience derivation: `successCount / (successCount + failureCount)`,
    /// `null` when the denominator is zero. Saves the client from a no-op
    /// division when nothing has run yet.
    pub success_rate: Option<f64>,
    pub outcomes: ResolutionOutcomeBreakdown,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceMetricsBucket {
    pub bucket_start: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub skipped_count: i64,
    pub fetched_sum: Option<i64>,
    pub new_sum: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceMetricsSummary {
    pub items: Vec<SourceMetricsSummaryItem>,
    pub range_seconds: i64,
    pub since: i64,
    pub until: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceMetricsDetail {
    pub source_name: String,
    pub summary: Option<SourceMetricsSummaryItem>,
    pub buckets: Vec<SourceMetricsBucket>,
    pub error_kinds: Vec<ErrorKindBucket>,
    pub fetch_latency: FetchLatencyDto,
    pub time_to_resolution: TimeToResolutionDto,
    pub bucket_seconds: i64,
    pub range_seconds: i64,
    pub since: i64,
    pub until: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMetricsSummaryItem {
    pub provider_id: String,
    pub total_runs: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub skipped_count: i64,
    pub bytes_sum: Option<i64>,
    pub last_started_at: Option<i64>,
    pub last_status: Option<String>,
    pub success_rate: Option<f64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMetricsBucket {
    pub bucket_start: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub skipped_count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMetricsSummary {
    pub items: Vec<ProviderMetricsSummaryItem>,
    pub range_seconds: i64,
    pub since: i64,
    pub until: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMetricsDetail {
    pub provider_id: String,
    pub summary: Option<ProviderMetricsSummaryItem>,
    pub buckets: Vec<ProviderMetricsBucket>,
    pub fetch_latency: FetchLatencyDto,
    pub bucket_seconds: i64,
    pub range_seconds: i64,
    pub since: i64,
    pub until: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIdMapCount {
    /// Canonical provider id (e.g. `mangaupdates`, `mal`, `anilist`).
    pub provider: String,
    /// Number of `(provider, external_id) → series_id` rows recorded.
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct MangaupdatesRedirectStats {
    /// Rows where `modern_id IS NOT NULL` — legacy ids we've successfully
    /// translated to a slug.
    pub modern_count: i64,
    /// Rows where `modern_id IS NULL` — legacy ids MU retired, so we no
    /// longer waste a HEAD request on them.
    pub tombstone_count: i64,
    /// Epoch seconds of the most recent translation. `None` when the
    /// cache is empty.
    pub last_resolved_at: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IdMapMetrics {
    /// Per-provider row counts from `series_external_ids`. Sorted
    /// alphabetically by provider for stable rendering.
    pub external_ids: Vec<ExternalIdMapCount>,
    /// State of the persisted MangaUpdates legacy → modern slug cache.
    pub mangaupdates_redirect_cache: MangaupdatesRedirectStats,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewQueueSnapshotDto {
    pub captured_at: i64,
    pub pending_count: i64,
    pub unresolved_count: i64,
    pub ambiguous_count: i64,
    pub review_pending_count: i64,
    pub oldest_pending_seconds: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewQueueMetrics {
    pub snapshots: Vec<ReviewQueueSnapshotDto>,
    /// Median time (seconds) from observed to resolved, over closures in
    /// `[since, until)`. `null` when nothing closed.
    pub time_to_decision_p50_seconds: Option<f64>,
    pub closed_count: i64,
    pub range_seconds: i64,
    pub since: i64,
    pub until: i64,
}

/// Across-all-sources summary over the requested window.
#[utoipa::path(
    get,
    path = "/api/v1/metrics/sources",
    tag = "metrics",
    operation_id = "metrics_sources_summary",
    params(MetricsQuery),
    responses((status = 200, body = SourceMetricsSummary))
)]
pub async fn sources_summary(
    State(state): State<AppState>,
    Query(q): Query<MetricsQuery>,
) -> ApiResult<Json<SourceMetricsSummary>> {
    let now = Utc::now().timestamp();
    let range_seconds = parse_range(q.range.as_deref());
    let since = now - range_seconds;
    let rows = run_metrics_repo::source_summary(&state.db, since, now)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(SourceMetricsSummary {
        items: rows.into_iter().map(map_source_summary).collect(),
        range_seconds,
        since,
        until: now,
    }))
}

/// Detailed metrics for a single source: summary + per-bucket counts.
#[utoipa::path(
    get,
    path = "/api/v1/metrics/sources/{name}",
    tag = "metrics",
    params(("name" = String, Path, description = "Source instance name"), MetricsQuery),
    responses((status = 200, body = SourceMetricsDetail))
)]
pub async fn sources_detail(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<MetricsQuery>,
) -> ApiResult<Json<SourceMetricsDetail>> {
    let now = Utc::now().timestamp();
    let range_seconds = parse_range(q.range.as_deref());
    let since = now - range_seconds;
    let buckets = clamp_buckets(q.buckets.unwrap_or(DEFAULT_BUCKETS));
    let bucket_seconds = ((range_seconds as f64) / (buckets as f64)).ceil() as i64;
    let bucket_seconds = bucket_seconds.max(1);

    let bucket_rows =
        run_metrics_repo::source_buckets(&state.db, &name, since, now, bucket_seconds)
            .await
            .map_err(ApiError::Internal)?;
    let summary_rows = run_metrics_repo::source_summary(&state.db, since, now)
        .await
        .map_err(ApiError::Internal)?;
    let summary = summary_rows
        .into_iter()
        .find(|r| r.source_name == name)
        .map(map_source_summary);

    let error_kinds = run_metrics_repo::source_error_kinds(&state.db, &name, since, now)
        .await
        .map_err(ApiError::Internal)?;
    let fetch = run_metrics_repo::source_fetch_latency(&state.db, &name, since, now)
        .await
        .map_err(ApiError::Internal)?;
    let ttr = run_metrics_repo::source_time_to_resolution(&state.db, &name, since, now)
        .await
        .map_err(ApiError::Internal)?;

    Ok(Json(SourceMetricsDetail {
        source_name: name,
        summary,
        buckets: bucket_rows
            .into_iter()
            .map(|b| SourceMetricsBucket {
                bucket_start: b.bucket_start,
                success_count: b.success_count,
                failure_count: b.failure_count,
                skipped_count: b.skipped_count,
                fetched_sum: b.fetched_sum,
                new_sum: b.new_sum,
            })
            .collect(),
        error_kinds: error_kinds
            .into_iter()
            .map(|r| ErrorKindBucket {
                kind: r.error_kind.unwrap_or_else(|| "unknown".to_string()),
                count: r.count,
            })
            .collect(),
        fetch_latency: FetchLatencyDto {
            p50_ms: fetch.p50_ms,
            p95_ms: fetch.p95_ms,
            max_ms: fetch.max_ms,
        },
        time_to_resolution: TimeToResolutionDto {
            p50_seconds: ttr.p50_seconds,
            p95_seconds: ttr.p95_seconds,
            count: ttr.count,
        },
        bucket_seconds,
        range_seconds,
        since,
        until: now,
    }))
}

/// Across-all-providers refresh summary.
#[utoipa::path(
    get,
    path = "/api/v1/metrics/providers",
    tag = "metrics",
    operation_id = "metrics_providers_summary",
    params(MetricsQuery),
    responses((status = 200, body = ProviderMetricsSummary))
)]
pub async fn providers_summary(
    State(state): State<AppState>,
    Query(q): Query<MetricsQuery>,
) -> ApiResult<Json<ProviderMetricsSummary>> {
    let now = Utc::now().timestamp();
    let range_seconds = parse_range(q.range.as_deref());
    let since = now - range_seconds;
    let rows = run_metrics_repo::provider_refresh_summary(&state.db, since, now)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(ProviderMetricsSummary {
        items: rows.into_iter().map(map_provider_summary).collect(),
        range_seconds,
        since,
        until: now,
    }))
}

/// Detailed refresh metrics for a single provider.
#[utoipa::path(
    get,
    path = "/api/v1/metrics/providers/{id}",
    tag = "metrics",
    params(("id" = String, Path, description = "Provider id"), MetricsQuery),
    responses((status = 200, body = ProviderMetricsDetail))
)]
pub async fn providers_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<MetricsQuery>,
) -> ApiResult<Json<ProviderMetricsDetail>> {
    let now = Utc::now().timestamp();
    let range_seconds = parse_range(q.range.as_deref());
    let since = now - range_seconds;
    let buckets = clamp_buckets(q.buckets.unwrap_or(DEFAULT_BUCKETS));
    let bucket_seconds = ((range_seconds as f64) / (buckets as f64)).ceil() as i64;
    let bucket_seconds = bucket_seconds.max(1);

    let bucket_rows =
        run_metrics_repo::provider_refresh_buckets(&state.db, &id, since, now, bucket_seconds)
            .await
            .map_err(ApiError::Internal)?;
    let summary_rows = run_metrics_repo::provider_refresh_summary(&state.db, since, now)
        .await
        .map_err(ApiError::Internal)?;
    let summary = summary_rows
        .into_iter()
        .find(|r| r.provider_id == id)
        .map(map_provider_summary);
    let fetch = run_metrics_repo::provider_fetch_latency(&state.db, &id, since, now)
        .await
        .map_err(ApiError::Internal)?;

    Ok(Json(ProviderMetricsDetail {
        provider_id: id,
        summary,
        buckets: bucket_rows
            .into_iter()
            .map(|b| ProviderMetricsBucket {
                bucket_start: b.bucket_start,
                success_count: b.success_count,
                failure_count: b.failure_count,
                skipped_count: b.skipped_count,
            })
            .collect(),
        fetch_latency: FetchLatencyDto {
            p50_ms: fetch.p50_ms,
            p95_ms: fetch.p95_ms,
            max_ms: fetch.max_ms,
        },
        bucket_seconds,
        range_seconds,
        since,
        until: now,
    }))
}

/// Review-queue depth-over-time + median time-to-decision for the window.
#[utoipa::path(
    get,
    path = "/api/v1/metrics/review-queue",
    tag = "metrics",
    params(MetricsQuery),
    responses((status = 200, body = ReviewQueueMetrics))
)]
pub async fn review_queue(
    State(state): State<AppState>,
    Query(q): Query<MetricsQuery>,
) -> ApiResult<Json<ReviewQueueMetrics>> {
    let now = Utc::now().timestamp();
    let range_seconds = parse_range(q.range.as_deref());
    let since = now - range_seconds;
    let snapshots = review_snapshots_repo::snapshots_between(&state.db, since, now)
        .await
        .map_err(ApiError::Internal)?;
    let decisions = review_snapshots_repo::time_to_decision(&state.db, since, now)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(ReviewQueueMetrics {
        snapshots: snapshots
            .into_iter()
            .map(|s| ReviewQueueSnapshotDto {
                captured_at: s.captured_at,
                pending_count: s.pending_count,
                unresolved_count: s.unresolved_count,
                ambiguous_count: s.ambiguous_count,
                review_pending_count: s.review_pending_count,
                oldest_pending_seconds: s.oldest_pending_seconds,
            })
            .collect(),
        time_to_decision_p50_seconds: decisions.p50_seconds,
        closed_count: decisions.count,
        range_seconds,
        since,
        until: now,
    }))
}

/// Foreign-id map sizes: `series_external_ids` counts grouped by provider,
/// plus persisted MangaUpdates legacy → modern slug cache stats. Used by
/// the admin "ID Maps" page; no range/bucketing applies because these
/// numbers are not time-series.
#[utoipa::path(
    get,
    path = "/api/v1/metrics/id-maps",
    tag = "metrics",
    operation_id = "metrics_id_maps",
    responses((status = 200, body = IdMapMetrics))
)]
pub async fn id_maps(State(state): State<AppState>) -> ApiResult<Json<IdMapMetrics>> {
    let external_rows = series_external_ids_repo::count_by_provider(&state.db)
        .await
        .map_err(ApiError::Internal)?;
    let mu_stats = mangaupdates_id_repo::stats(&state.db)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(IdMapMetrics {
        external_ids: external_rows
            .into_iter()
            .map(|r| ExternalIdMapCount {
                provider: r.provider,
                count: r.count,
            })
            .collect(),
        mangaupdates_redirect_cache: MangaupdatesRedirectStats {
            modern_count: mu_stats.modern_count,
            tombstone_count: mu_stats.tombstone_count,
            last_resolved_at: mu_stats.last_resolved_at,
        },
    }))
}

fn map_source_summary(r: run_metrics_repo::SourceSummaryRow) -> SourceMetricsSummaryItem {
    SourceMetricsSummaryItem {
        success_rate: success_rate(r.success_count, r.failure_count),
        outcomes: ResolutionOutcomeBreakdown {
            known_id: r.outcome_known_id_sum.unwrap_or(0),
            foreign_id: r.outcome_foreign_id_sum.unwrap_or(0),
            fuzzy: r.outcome_fuzzy_sum.unwrap_or(0),
            review: r.outcome_review_sum.unwrap_or(0),
            failed: r.outcome_failed_sum.unwrap_or(0),
        },
        source_name: r.source_name,
        total_runs: r.total_runs,
        success_count: r.success_count,
        failure_count: r.failure_count,
        skipped_count: r.skipped_count,
        fetched_sum: r.fetched_sum,
        new_sum: r.new_sum,
        resolved_sum: r.resolved_sum,
        enrich_duration_ms_sum: r.enrich_duration_ms_sum,
        resolve_duration_ms_sum: r.resolve_duration_ms_sum,
        last_started_at: r.last_started_at,
        last_status: r.last_status,
    }
}

fn map_provider_summary(
    r: run_metrics_repo::ProviderRefreshSummaryRow,
) -> ProviderMetricsSummaryItem {
    ProviderMetricsSummaryItem {
        success_rate: success_rate(r.success_count, r.failure_count),
        provider_id: r.provider_id,
        total_runs: r.total_runs,
        success_count: r.success_count,
        failure_count: r.failure_count,
        skipped_count: r.skipped_count,
        bytes_sum: r.bytes_sum,
        last_started_at: r.last_started_at,
        last_status: r.last_status,
    }
}

fn parse_range(input: Option<&str>) -> i64 {
    let Some(raw) = input else {
        return DEFAULT_RANGE_SECONDS;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return DEFAULT_RANGE_SECONDS;
    }
    let (num_part, unit) = match trimmed.chars().last() {
        Some(c) if c.is_ascii_alphabetic() => (&trimmed[..trimmed.len() - c.len_utf8()], c),
        _ => return DEFAULT_RANGE_SECONDS,
    };
    let Ok(n) = num_part.parse::<i64>() else {
        return DEFAULT_RANGE_SECONDS;
    };
    let seconds = match unit.to_ascii_lowercase() {
        's' => n,
        'm' => n * 60,
        'h' => n * 60 * 60,
        'd' => n * 24 * 60 * 60,
        'w' => n * 7 * 24 * 60 * 60,
        _ => return DEFAULT_RANGE_SECONDS,
    };
    // Clamp to sensible bounds: 1 minute … 90 days.
    seconds.clamp(60, 90 * 24 * 60 * 60)
}

fn clamp_buckets(input: u32) -> u32 {
    input.clamp(MIN_BUCKETS, MAX_BUCKETS)
}

fn success_rate(success: i64, failure: i64) -> Option<f64> {
    let total = success + failure;
    if total <= 0 {
        return None;
    }
    Some(success as f64 / total as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_range_defaults_when_missing_or_malformed() {
        assert_eq!(parse_range(None), DEFAULT_RANGE_SECONDS);
        assert_eq!(parse_range(Some("")), DEFAULT_RANGE_SECONDS);
        assert_eq!(parse_range(Some("garbage")), DEFAULT_RANGE_SECONDS);
        assert_eq!(parse_range(Some("12")), DEFAULT_RANGE_SECONDS);
    }

    #[test]
    fn parse_range_handles_basic_units() {
        assert_eq!(parse_range(Some("90s")), 90);
        assert_eq!(parse_range(Some("5m")), 300);
        assert_eq!(parse_range(Some("24h")), 24 * 60 * 60);
        assert_eq!(parse_range(Some("7d")), 7 * 24 * 60 * 60);
        assert_eq!(parse_range(Some("2w")), 2 * 7 * 24 * 60 * 60);
    }

    #[test]
    fn parse_range_clamps_extremes() {
        assert_eq!(parse_range(Some("1s")), 60);
        assert_eq!(parse_range(Some("365d")), 90 * 24 * 60 * 60);
    }
}
