import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { components } from "@/types/api.generated";

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

export interface SeriesFilters {
  kind?: string;
  status?: string;
  owned?: boolean;
  genre?: string;
  tag?: string;
  sort?: string;
  order?: string;
  page?: number;
  pageSize?: number;
}

const DEFAULT_PAGE_SIZE = 24;

export function useSeriesList(filters: SeriesFilters) {
  const query = {
    page: filters.page ?? 1,
    pageSize: filters.pageSize ?? DEFAULT_PAGE_SIZE,
    kind: filters.kind || undefined,
    status: filters.status || undefined,
    owned: typeof filters.owned === "boolean" ? filters.owned : undefined,
    genre: filters.genre || undefined,
    tag: filters.tag || undefined,
    sort: filters.sort || undefined,
    order: filters.order || undefined,
  };
  return useQuery({
    queryKey: ["series-list", query],
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
  return useQuery({
    queryKey: ["series-detail", id],
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

export function useUnresolvedReleases(
  page = 1,
  pageSize = DEFAULT_REVIEW_PAGE_SIZE,
) {
  return useQuery({
    queryKey: ["releases-unresolved", page, pageSize],
    queryFn: async () => {
      const { data, error } = await api.GET("/api/v1/releases/unresolved", {
        params: { query: { page, pageSize } },
      });
      if (error) throw new Error("failed to load review queue");
      return data;
    },
    placeholderData: (prev) => prev,
  });
}
