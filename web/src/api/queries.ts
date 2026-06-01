import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import { useAdminAuth } from "@/stores/auth";
import { DEFAULT_PAGE_SIZE } from "@/stores/uiPrefs";
import type { components } from "@/types/api.generated";

export type AppInfo = components["schemas"]["AppInfo"];
export type CodexStatusDto = components["schemas"]["CodexStatusDto"];
export type CodexInfo = components["schemas"]["CodexInfo"];
export type SeriesListItem = components["schemas"]["SeriesListItem"];
export type SeriesListPage = components["schemas"]["SeriesListPage"];
export type SeriesDetail = components["schemas"]["SeriesDetail"];
export type ReleaseDto = components["schemas"]["ReleaseDto"];
export type ReleasePage = components["schemas"]["ReleasePage"];
export type SourceDto = components["schemas"]["SourceDto"];
export type SourceConfigDto = components["schemas"]["SourceConfigDto"];
export type ProviderDto = components["schemas"]["ProviderDto"];
export type ProviderConfigDto = components["schemas"]["ProviderConfigDto"];
export type PollAllResponse = components["schemas"]["PollAllResponse"];
export type RefreshAllResponse = components["schemas"]["RefreshAllResponse"];
export type StatsResponse = components["schemas"]["StatsResponse"];
export type UnresolvedPage = components["schemas"]["UnresolvedPage"];
export type UnresolvedRelease = components["schemas"]["UnresolvedRelease"];
export type ReviewCandidateDto = components["schemas"]["ReviewCandidateDto"];
export type ProviderSearchHit = components["schemas"]["ProviderSearchHit"];
export type ProviderSearchResponse =
  components["schemas"]["ProviderSearchResponse"];
export type TagList = components["schemas"]["TagList"];
export type TagUsageDto = components["schemas"]["TagUsageDto"];
export type SourceMetricsSummary =
  components["schemas"]["SourceMetricsSummary"];
export type SourceMetricsSummaryItem =
  components["schemas"]["SourceMetricsSummaryItem"];
export type SourceMetricsDetail = components["schemas"]["SourceMetricsDetail"];
export type SourceMetricsBucket = components["schemas"]["SourceMetricsBucket"];
export type ProviderMetricsSummary =
  components["schemas"]["ProviderMetricsSummary"];
export type ProviderMetricsSummaryItem =
  components["schemas"]["ProviderMetricsSummaryItem"];
export type ProviderMetricsDetail =
  components["schemas"]["ProviderMetricsDetail"];
export type ResolutionOutcomeBreakdown =
  components["schemas"]["ResolutionOutcomeBreakdown"];
export type ErrorKindBucket = components["schemas"]["ErrorKindBucket"];
export type FetchLatencyDto = components["schemas"]["FetchLatencyDto"];
export type TimeToResolutionDto = components["schemas"]["TimeToResolutionDto"];
export type ReviewQueueMetrics = components["schemas"]["ReviewQueueMetrics"];
export type ReviewQueueSnapshotDto =
  components["schemas"]["ReviewQueueSnapshotDto"];
export type IdMapMetrics = components["schemas"]["IdMapMetrics"];
export type ExternalIdMapCount = components["schemas"]["ExternalIdMapCount"];
export type MangaupdatesRedirectStats =
  components["schemas"]["MangaupdatesRedirectStats"];
export type ProviderMetricsBucket =
  components["schemas"]["ProviderMetricsBucket"];

export interface SeriesFilters {
  kind?: string;
  status?: string;
  owned?: boolean;
  /// `true` keeps only series with ≥1 linked release; `false` keeps only
  /// orphans. Absent means "no constraint". Mirrors the backend filter.
  hasReleases?: boolean;
  /// Selected genre names. Joined into a CSV before being sent — the
  /// backend re-splits on the comma.
  genres?: string[];
  genresMode?: "any" | "all";
  tags?: string[];
  tagsMode?: "any" | "all";
  /// Metadata provenance filter: `manual` keeps only operator-authored
  /// series, `auto` keeps only provider-backed ones. Mirrors the backend.
  metadataSource?: "manual" | "auto";
  sort?: string;
  order?: string;
  page?: number;
  pageSize?: number;
  /// Free-text search query. Whitespace-only is treated as absent so the
  /// server avoids the rerank pass for an effectively-empty query.
  q?: string;
  /// Codex presence filter (`any` | `missing` | `complete` | `behind` |
  /// `present`). Admin-only and enforced server-side; ignored for non-admins.
  codexStatus?: string;
}

export function useSeriesList(filters: SeriesFilters) {
  // The series payload carries the admin-only `codex` overlay when a valid
  // admin token is present, so the cache key must distinguish admin from anon
  // responses — otherwise logging in/out would serve a stale payload.
  const hasAdmin = useAdminAuth((s) => Boolean(s.token));
  const trimmedQ = filters.q?.trim();
  const genresCsv = filters.genres?.length
    ? filters.genres.join(",")
    : undefined;
  const tagsCsv = filters.tags?.length ? filters.tags.join(",") : undefined;
  const query = {
    page: filters.page ?? 1,
    pageSize: filters.pageSize ?? DEFAULT_PAGE_SIZE,
    kind: filters.kind || undefined,
    status: filters.status || undefined,
    owned: typeof filters.owned === "boolean" ? filters.owned : undefined,
    hasReleases:
      typeof filters.hasReleases === "boolean"
        ? filters.hasReleases
        : undefined,
    genres: genresCsv,
    // The backend defaults to `any`; only send a mode when there's a
    // selection to scope it to, and never when the mode is already the
    // default (keeps the URL/cache key clean).
    genresMode: genresCsv && filters.genresMode === "all" ? "all" : undefined,
    tags: tagsCsv,
    tagsMode: tagsCsv && filters.tagsMode === "all" ? "all" : undefined,
    metadataSource: filters.metadataSource || undefined,
    sort: filters.sort || undefined,
    order: filters.order || undefined,
    q: trimmedQ || undefined,
    // Backend drops this for non-admins; send it regardless and let the
    // server enforce. Keeps the URL/cache key honest for admins.
    codexStatus: filters.codexStatus || undefined,
  };
  return useQuery({
    queryKey: ["series-list", query, { admin: hasAdmin }],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/series", {
        params: { query },
      });
      if (error) throw new Error("failed to load series");
      return data;
    },
    placeholderData: (prev) => prev,
  });
}

export function useSeriesDetail(id: number | undefined) {
  const hasAdmin = useAdminAuth((s) => Boolean(s.token));
  return useQuery({
    queryKey: ["series-detail", id, { admin: hasAdmin }],
    enabled: typeof id === "number" && Number.isFinite(id),
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/series/{id}", {
        params: { path: { id: id as number } },
      });
      if (error) throw new Error("failed to load series");
      return data;
    },
  });
}

/// Codex connection-health status for the admin maintenance page. Admin-only
/// on the server; disabled here unless a token is present so anon sessions
/// don't fire it.
export function useCodexStatus() {
  const hasAdmin = useAdminAuth((s) => Boolean(s.token));
  return useQuery({
    queryKey: ["codex-status", { admin: hasAdmin }],
    enabled: hasAdmin,
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/codex/status");
      if (error) throw new Error("failed to load codex status");
      return data;
    },
    staleTime: 30_000,
  });
}

export function useSeriesReleases(
  seriesId: number | undefined,
  page = 1,
  pageSize = 50,
) {
  return useQuery({
    queryKey: ["series-releases", seriesId, page, pageSize],
    enabled: typeof seriesId === "number" && Number.isFinite(seriesId),
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/releases", {
        params: {
          query: {
            seriesId: seriesId as number,
            page,
            pageSize,
          },
        },
      });
      if (error) throw new Error("failed to load releases");
      return data;
    },
    placeholderData: (prev) => prev,
  });
}

export function useSources() {
  return useQuery({
    queryKey: ["sources"],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/sources");
      if (error) throw new Error("failed to load sources");
      return data;
    },
    staleTime: 60_000,
  });
}

/// App name + semver. The value can't change without a fresh page load
/// (the binary would have to be redeployed and the SPA reloaded), so
/// fetch once per session and never refetch.
export function useAppInfo() {
  return useQuery({
    queryKey: ["app-info"],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/info");
      if (error) throw new Error("failed to load app info");
      return data;
    },
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: Number.POSITIVE_INFINITY,
    refetchOnWindowFocus: false,
    retry: 1,
  });
}

export function useStats() {
  return useQuery({
    queryKey: ["stats"],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/stats");
      if (error) throw new Error("failed to load stats");
      return data;
    },
    staleTime: 30_000,
  });
}

export function useProviders() {
  return useQuery({
    queryKey: ["providers"],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/providers");
      if (error) throw new Error("failed to load providers");
      return data;
    },
    staleTime: 60_000,
  });
}

/// Search a single provider by title or external ID. Disabled until both
/// the provider id and at least one of (q, externalId) are non-empty —
/// keeps the modal from firing speculative network requests as the
/// operator types the very first character.
export function useProviderSearch(opts: {
  providerId: string | null;
  q: string;
  externalId: string;
  /**
   * Canonical id of the provider the `externalId` belongs to, when it's a
   * foreign id resolved through `providerId` (e.g. a MangaUpdates id against
   * MangaBaka). Only sent on the externalId path.
   */
  foreignProvider?: string;
  enabled?: boolean;
  /** Server-side cap. Default 50; the backend currently allows up to 100. */
  limit?: number;
}) {
  const trimmedQ = opts.q.trim();
  const trimmedExt = opts.externalId.trim();
  const foreignProvider = opts.foreignProvider?.trim() || undefined;
  const hasInput = Boolean(trimmedQ || trimmedExt);
  const enabled =
    Boolean(opts.providerId) && hasInput && (opts.enabled ?? true);
  const limit = opts.limit ?? 50;
  return useQuery({
    queryKey: [
      "provider-search",
      opts.providerId,
      trimmedQ,
      trimmedExt,
      foreignProvider,
      limit,
    ],
    enabled,
    queryFn: async () => {
      const params: {
        q?: string;
        externalId?: string;
        foreignProvider?: string;
        limit?: number;
      } = { limit };
      if (trimmedExt) {
        params.externalId = trimmedExt;
        if (foreignProvider) params.foreignProvider = foreignProvider;
      } else if (trimmedQ) {
        params.q = trimmedQ;
      }
      const { data, error } = await api.GET("/api/v1/providers/{id}/search", {
        params: {
          path: { id: opts.providerId as string },
          query: params,
        },
      });
      if (error) throw new Error("provider search failed");
      return data;
    },
    staleTime: 30_000,
  });
}

const DEFAULT_REVIEW_PAGE_SIZE = 20;

export function useGenres() {
  return useQuery({
    queryKey: ["genres"],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/genres");
      if (error) throw new Error("failed to load genres");
      return data;
    },
    staleTime: 60_000,
  });
}

export function useTags() {
  return useQuery({
    queryKey: ["tags"],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/tags");
      if (error) throw new Error("failed to load tags");
      return data;
    },
    staleTime: 60_000,
  });
}

export interface MetricsRange {
  range?: string;
  buckets?: number;
}

export function useSourceMetricsSummary(opts: MetricsRange = {}) {
  const query = {
    range: opts.range || undefined,
    buckets: typeof opts.buckets === "number" ? opts.buckets : undefined,
  };
  return useQuery({
    queryKey: ["metrics-sources", query],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/metrics/sources", {
        params: { query },
      });
      if (error) throw new Error("failed to load source metrics");
      return data;
    },
    staleTime: 30_000,
  });
}

export function useSourceMetricsDetail(
  name: string | undefined,
  opts: MetricsRange = {},
) {
  const query = {
    range: opts.range || undefined,
    buckets: typeof opts.buckets === "number" ? opts.buckets : undefined,
  };
  return useQuery({
    queryKey: ["metrics-source", name, query],
    enabled: typeof name === "string" && name.length > 0,
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/metrics/sources/{name}", {
        params: { path: { name: name as string }, query },
      });
      if (error) throw new Error("failed to load source metrics detail");
      return data;
    },
    staleTime: 30_000,
  });
}

export function useProviderMetricsDetail(
  id: string | undefined,
  opts: MetricsRange = {},
) {
  const query = {
    range: opts.range || undefined,
    buckets: typeof opts.buckets === "number" ? opts.buckets : undefined,
  };
  return useQuery({
    queryKey: ["metrics-provider", id, query],
    enabled: typeof id === "string" && id.length > 0,
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/metrics/providers/{id}", {
        params: { path: { id: id as string }, query },
      });
      if (error) throw new Error("failed to load provider metrics detail");
      return data;
    },
    staleTime: 30_000,
  });
}

export function useReviewQueueMetrics(opts: MetricsRange = {}) {
  const query = {
    range: opts.range || undefined,
  };
  return useQuery({
    queryKey: ["metrics-review-queue", query],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/metrics/review-queue", {
        params: { query },
      });
      if (error) throw new Error("failed to load review-queue metrics");
      return data;
    },
    staleTime: 30_000,
  });
}

export function useProviderMetricsSummary(opts: MetricsRange = {}) {
  const query = {
    range: opts.range || undefined,
    buckets: typeof opts.buckets === "number" ? opts.buckets : undefined,
  };
  return useQuery({
    queryKey: ["metrics-providers", query],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/metrics/providers", {
        params: { query },
      });
      if (error) throw new Error("failed to load provider metrics");
      return data;
    },
    staleTime: 30_000,
  });
}

export function useIdMapMetrics() {
  return useQuery({
    queryKey: ["metrics-id-maps"],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/metrics/id-maps");
      if (error) throw new Error("failed to load id-map metrics");
      return data;
    },
    staleTime: 60_000,
  });
}

export interface ReviewFilters {
  page?: number;
  pageSize?: number;
  /// Free-text title search. Whitespace-only is treated as absent.
  q?: string;
  sourceName?: string;
  format?: string;
  /// One of the queue statuses (`unresolved` / `ambiguous` /
  /// `review_pending`); anything else is clamped server-side.
  status?: string;
  /// Release-group filter: the cleaned search query a clicked group hands off.
  /// Kept distinct from the free-text title `q`; the two AND together.
  searchQuery?: string;
  /// Grouping breadth for `searchQuery` (1 = primary query only, default; 2 =
  /// first two; 3 = all). Ignored unless `searchQuery` is set.
  breadth?: number;
  /// Result ordering. One of `observed_desc` (default) / `observed_asc` /
  /// `posted_desc` / `posted_asc` / `title_asc` / `title_desc`; anything
  /// else falls back to `observed_desc` server-side.
  sort?: string;
}

export function useUnresolvedReleases(filters: ReviewFilters = {}) {
  const trimmedQ = filters.q?.trim();
  const trimmedSearchQuery = filters.searchQuery?.trim();
  const query = {
    page: filters.page ?? 1,
    pageSize: filters.pageSize ?? DEFAULT_REVIEW_PAGE_SIZE,
    q: trimmedQ || undefined,
    sourceName: filters.sourceName || undefined,
    format: filters.format || undefined,
    status: filters.status || undefined,
    searchQuery: trimmedSearchQuery || undefined,
    // Breadth only affects the result set together with searchQuery; omit it
    // otherwise so toggling the grouping looseness doesn't refetch the list.
    breadth: trimmedSearchQuery ? (filters.breadth ?? 1) : undefined,
    sort: filters.sort || undefined,
  };
  return useQuery({
    queryKey: ["releases-unresolved", query],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/releases/unresolved", {
        params: { query },
      });
      if (error) throw new Error("failed to load review queue");
      return data;
    },
    placeholderData: (prev) => prev,
  });
}

/// Filters for the grouped review-queue endpoint. Mirrors the list filters
/// minus pagination/sort and the group filter itself (a group within a group
/// is meaningless — the clusters are computed over the non-group-scoped set).
export interface ReleaseGroupFilters {
  q?: string;
  sourceName?: string;
  format?: string;
  status?: string;
  breadth?: number;
}

/// Cluster the review queue by cleaned search query at the given breadth.
/// `enabled` gates the fetch so a collapsed panel doesn't hit the endpoint.
export function useReleaseGroups(
  filters: ReleaseGroupFilters = {},
  enabled = true,
) {
  const trimmedQ = filters.q?.trim();
  const query = {
    q: trimmedQ || undefined,
    sourceName: filters.sourceName || undefined,
    format: filters.format || undefined,
    status: filters.status || undefined,
    breadth: filters.breadth ?? 1,
  };
  return useQuery({
    queryKey: ["releases-unresolved-groups", query],
    queryFn: async () => {
      const { data, error } = await api.GET(
        "/api/v1/releases/unresolved/groups",
        { params: { query } },
      );
      if (error) throw new Error("failed to load release groups");
      return data;
    },
    enabled,
    placeholderData: (prev) => prev,
  });
}

/// Releases the operator marked `standalone` — worthwhile one-shots (a
/// guidebook, an artbook) that are deliberately not tracked as a series.
/// Backed by the generic release list filtered to `status=standalone`.
export function useKeptReleases(page = 1, pageSize = DEFAULT_REVIEW_PAGE_SIZE) {
  return useQuery({
    queryKey: ["releases-kept", page, pageSize],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/releases", {
        params: { query: { status: "standalone", page, pageSize } },
      });
      if (error) throw new Error("failed to load kept releases");
      return data;
    },
    placeholderData: (prev) => prev,
  });
}
