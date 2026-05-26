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

export interface SeriesFilters {
  kind?: string;
  status?: string;
  owned?: boolean;
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
