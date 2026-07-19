//! OpenAPI specification.
//!
//! Every handler is listed in `paths(...)` and every DTO in
//! `components(schemas(...))`. utoipa derives the rest from the handler
//! attributes. Frontend codegen (`openapi-typescript`) reads this spec via
//! `tsundoku openapi`.

use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

use crate::codex_presence::{CodexInfo, CodexLinkKind, CodexStatus};
use crate::errors::ApiErrorBody;
use crate::handlers::codex::{
    CodexLinkRequest, CodexLinkResponse, CodexRefreshResponse, CodexStatusDto, SyncRunDto,
};
use crate::handlers::covers::InvalidateCoverCacheResponse;
use crate::handlers::download::{
    DownloadStatusDto, HealthCheckDto, SendRecordDto, SendToClientRequest,
};
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
    BulkLinkRequest, BulkLinkResponse, BulkRejectResponse, BulkRetryResponse, BulkReviewRequest,
    ExtractedLinksDto, LinkRequest, ReleaseDto, ReleaseGroupCandidateDto, ReleaseGroupDto,
    ReleaseGroupsResponse, ReleasePage, RetryAllResponse, ReviewCandidateDto, UnresolvedPage,
    UnresolvedRelease,
};
use crate::handlers::search::{
    BulkSearchReleasesRequest, BulkSearchReleasesResponse, GlobalSearchRunDto,
    GlobalSearchRunsResponse, SearchEntriesResponse, SearchEntryDto, SearchReleasesRequest,
    SearchReleasesResponse, SearchRunDto, SearchRunsResponse,
};
use crate::handlers::series::{
    BulkRefreshMetadataRequest, BulkRefreshMetadataResponse, BulkRefreshSkipDto,
    BulkWishlistRequest, BulkWishlistResponse, CoverageSpanDto, CreateSeriesFromProviderRequest,
    CreateSeriesRequest, ExternalIdDto, InvalidateMetadataHashesResponse, RecomputeSpansResponse,
    RefreshAllSeriesResponse, SeriesDetail, SeriesFeedItem, SeriesFeedRequest, SeriesFeedResponse,
    SeriesListItem, SeriesListPage, SeriesLookupResponse, SetIgnoreCompletionRequest,
    SetWishlistedRequest, UpdateSeriesRequest,
};
use crate::handlers::sources::{
    ManualBackfillResponse, ManualPollResponse, ManualReenrichResponse, PollAllResponse,
    ReenrichRequest, SourceConfigDto, SourceDto, SourceList, SourceRunDto, SourceRunsResponse,
};
use crate::handlers::stats::{ReleaseCounts, StatsResponse};
use crate::handlers::tagging::{TagList, TagUsageDto};
use crate::handlers::{
    codex, covers, download, events, health, info, metrics, providers, releases, search, series,
    series_export, sources, stats, tagging,
};
use crate::state::{InFlight, JobEvent, JobKind, JobPhase, JobProgress, JobResult};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "tsundoku API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Manga discovery service. Read endpoints (`GET`) are public unless `auth.read_requires_auth = true`. Write endpoints (`POST`) always require the admin bearer."
    ),
    paths(
        health::health,
        info::info,
        stats::stats,
        series::list,
        series::get,
        series::lookup,
        series::feed,
        series::feed_query,
        series::create,
        series::create_from_provider,
        series::update,
        series::refresh_metadata,
        series::set_ignore_completion,
        series::set_wishlisted,
        series::bulk_wishlist,
        series::bulk_refresh_metadata,
        series::refresh_all,
        series::recompute_spans,
        series::invalidate_metadata_hashes,
        series_export::export,
        releases::list,
        releases::list_unresolved,
        releases::list_groups,
        releases::link,
        releases::reject,
        releases::keep,
        releases::retry,
        releases::retry_all,
        releases::bulk_reject,
        releases::bulk_retry,
        releases::bulk_link,
        sources::list,
        sources::list_with_series_counts,
        sources::poll,
        sources::backfill,
        sources::reenrich,
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
        covers::get_by_series_id,
        covers::get_by_url,
        covers::invalidate_cache,
        codex::refresh,
        codex::status,
        codex::test,
        codex::link,
        codex::unlink,
        download::send,
        download::status,
        download::test,
        search::entries,
        search::trigger,
        search::bulk_trigger,
        search::runs,
        search::global_runs,
        sources::runs,
    ),
    components(schemas(
        ApiErrorBody,
        SearchEntryDto,
        SearchEntriesResponse,
        SearchReleasesRequest,
        SearchReleasesResponse,
        BulkSearchReleasesRequest,
        BulkSearchReleasesResponse,
        SearchRunDto,
        SearchRunsResponse,
        GlobalSearchRunDto,
        GlobalSearchRunsResponse,
        SourceRunDto,
        SourceRunsResponse,
        health::Health,
        info::AppInfo,
        StatsResponse,
        ReleaseCounts,
        SeriesListItem,
        SeriesListPage,
        SeriesDetail,
        SeriesLookupResponse,
        SeriesFeedItem,
        SeriesFeedResponse,
        SeriesFeedRequest,
        CoverageSpanDto,
        CreateSeriesRequest,
        CreateSeriesFromProviderRequest,
        UpdateSeriesRequest,
        SetIgnoreCompletionRequest,
        SetWishlistedRequest,
        BulkWishlistRequest,
        BulkWishlistResponse,
        BulkRefreshMetadataRequest,
        BulkRefreshMetadataResponse,
        BulkRefreshSkipDto,
        RefreshAllSeriesResponse,
        RecomputeSpansResponse,
        InvalidateMetadataHashesResponse,
        ExternalIdDto,
        ReleaseDto,
        ReleasePage,
        UnresolvedRelease,
        UnresolvedPage,
        ReleaseGroupDto,
        ReleaseGroupCandidateDto,
        ReleaseGroupsResponse,
        ReviewCandidateDto,
        ExtractedLinksDto,
        RetryAllResponse,
        BulkReviewRequest,
        BulkRejectResponse,
        BulkRetryResponse,
        BulkLinkRequest,
        BulkLinkResponse,
        LinkRequest,
        SourceDto,
        SourceConfigDto,
        SourceList,
        ManualPollResponse,
        ManualBackfillResponse,
        ManualReenrichResponse,
        ReenrichRequest,
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
        JobProgress,
        InFlight,
        InvalidateCoverCacheResponse,
        CodexRefreshResponse,
        CodexStatusDto,
        SyncRunDto,
        CodexLinkRequest,
        CodexLinkResponse,
        CodexInfo,
        CodexStatus,
        CodexLinkKind,
        SendToClientRequest,
        DownloadStatusDto,
        HealthCheckDto,
        SendRecordDto,
    )),
    tags(
        (name = "system", description = "Health and aggregate counters"),
        (name = "series", description = "Resolved series catalog"),
        (name = "releases", description = "Raw release feed and review queue"),
        (name = "sources", description = "Discovery-source state and triggers"),
        (name = "providers", description = "Metadata-provider state and triggers"),
        (name = "tagging", description = "Canonical genre and tag lists for filter UI"),
        (name = "metrics", description = "Per-source / per-provider historical run metrics"),
        (name = "covers", description = "Cover-image proxy and on-disk cache control"),
        (name = "codex", description = "Codex presence integration (admin-only)"),
        (name = "download", description = "Send to torrent client (admin-only)"),
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
