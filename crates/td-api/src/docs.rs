//! OpenAPI specification.
//!
//! Every handler is listed in `paths(...)` and every DTO in
//! `components(schemas(...))`. utoipa derives the rest from the handler
//! attributes. Frontend codegen (`openapi-typescript`) reads this spec via
//! `tsundoku openapi`.

use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

use crate::errors::ApiErrorBody;
use crate::handlers::metrics::{
    ErrorKindBucket, ExternalIdMapCount, FetchLatencyDto, IdMapMetrics, MangaupdatesRedirectStats,
    ProviderMetricsBucket, ProviderMetricsDetail, ProviderMetricsSummary,
    ProviderMetricsSummaryItem, ResolutionOutcomeBreakdown, ReviewQueueMetrics,
    ReviewQueueSnapshotDto, SourceMetricsBucket, SourceMetricsDetail, SourceMetricsSummary,
    SourceMetricsSummaryItem, TimeToResolutionDto,
};
use crate::handlers::providers::{
    ProviderCacheState, ProviderConfigDto, ProviderDto, ProviderList, ProviderSearchHit,
    ProviderSearchResponse, RefreshAllResponse, RefreshResponse,
};
use crate::handlers::releases::{
    BulkRejectResponse, BulkRetryResponse, BulkReviewRequest, ExtractedLinksDto, LinkRequest,
    ReleaseDto, ReleasePage, RetryAllResponse, ReviewCandidateDto, UnresolvedPage,
    UnresolvedRelease,
};
use crate::handlers::series::{
    CreateSeriesRequest, ExternalIdDto, RefreshAllSeriesResponse, SeriesDetail, SeriesListItem,
    SeriesListPage,
};
use crate::handlers::sources::{
    ManualBackfillResponse, ManualPollResponse, PollAllResponse, SourceConfigDto, SourceDto,
    SourceList,
};
use crate::handlers::stats::{ReleaseCounts, StatsResponse};
use crate::handlers::tagging::{TagList, TagUsageDto};
use crate::handlers::{
    events, health, metrics, providers, releases, series, sources, stats, tagging,
};
use crate::state::{JobEvent, JobKind, JobPhase, JobResult};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "tsundoku API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Manga discovery service. Read endpoints (`GET`) are public unless `auth.read_requires_auth = true`. Write endpoints (`POST`) always require the admin bearer."
    ),
    paths(
        health::health,
        stats::stats,
        series::list,
        series::get,
        series::create,
        series::refresh_metadata,
        series::refresh_all,
        releases::list,
        releases::list_unresolved,
        releases::link,
        releases::reject,
        releases::keep,
        releases::retry,
        releases::retry_all,
        releases::bulk_reject,
        releases::bulk_retry,
        sources::list,
        sources::poll,
        sources::backfill,
        sources::poll_all,
        providers::list,
        providers::refresh_cache,
        providers::refresh_all,
        providers::search,
        tagging::list_genres,
        tagging::list_tags,
        metrics::sources_summary,
        metrics::sources_detail,
        metrics::providers_summary,
        metrics::providers_detail,
        metrics::review_queue,
        metrics::id_maps,
        events::jobs,
    ),
    components(schemas(
        ApiErrorBody,
        health::Health,
        StatsResponse,
        ReleaseCounts,
        SeriesListItem,
        SeriesListPage,
        SeriesDetail,
        CreateSeriesRequest,
        RefreshAllSeriesResponse,
        ExternalIdDto,
        ReleaseDto,
        ReleasePage,
        UnresolvedRelease,
        UnresolvedPage,
        ReviewCandidateDto,
        ExtractedLinksDto,
        RetryAllResponse,
        BulkReviewRequest,
        BulkRejectResponse,
        BulkRetryResponse,
        LinkRequest,
        SourceDto,
        SourceConfigDto,
        SourceList,
        ManualPollResponse,
        ManualBackfillResponse,
        PollAllResponse,
        ProviderDto,
        ProviderConfigDto,
        ProviderList,
        ProviderCacheState,
        RefreshResponse,
        RefreshAllResponse,
        ProviderSearchHit,
        ProviderSearchResponse,
        TagUsageDto,
        TagList,
        SourceMetricsSummary,
        SourceMetricsSummaryItem,
        SourceMetricsBucket,
        SourceMetricsDetail,
        ProviderMetricsSummary,
        ProviderMetricsSummaryItem,
        ProviderMetricsBucket,
        ProviderMetricsDetail,
        ResolutionOutcomeBreakdown,
        FetchLatencyDto,
        TimeToResolutionDto,
        ErrorKindBucket,
        ReviewQueueSnapshotDto,
        ReviewQueueMetrics,
        ExternalIdMapCount,
        MangaupdatesRedirectStats,
        IdMapMetrics,
        JobEvent,
        JobKind,
        JobPhase,
        JobResult,
    )),
    tags(
        (name = "system", description = "Health and aggregate counters"),
        (name = "series", description = "Resolved series catalog"),
        (name = "releases", description = "Raw release feed and review queue"),
        (name = "sources", description = "Discovery-source state and triggers"),
        (name = "providers", description = "Metadata-provider state and triggers"),
        (name = "tagging", description = "Canonical genre and tag lists for filter UI"),
        (name = "metrics", description = "Per-source / per-provider historical run metrics")
    ),
    modifiers(&BearerSecurity)
)]
pub struct ApiDoc;

struct BearerSecurity;

impl utoipa::Modify for BearerSecurity {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "admin",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("opaque")
                    .description(Some("Admin bearer token (auth.admin_token in config)."))
                    .build(),
            ),
        );
    }
}
