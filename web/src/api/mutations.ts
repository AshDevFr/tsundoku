import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { components } from "@/types/api.generated";

type LinkRequest = components["schemas"]["LinkRequest"];

// Extract a useful error message from an openapi-fetch error payload, falling
// back to a sentence the user can act on. The backend serializes errors as
// `{ error, message }` (see crates/td-api/src/errors.rs).
function describeError(error: unknown, fallback: string): string {
  if (error && typeof error === "object") {
    const e = error as { message?: unknown; error?: unknown };
    if (typeof e.message === "string" && e.message) return e.message;
    if (typeof e.error === "string" && e.error) return e.error;
  }
  return fallback;
}

function invalidateReleaseQueries(qc: ReturnType<typeof useQueryClient>) {
  qc.invalidateQueries({ queryKey: ["releases-unresolved"] });
  qc.invalidateQueries({ queryKey: ["stats"] });
  qc.invalidateQueries({ queryKey: ["series-list"] });
}

export function useLinkRelease() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (args: { releaseId: string; body: LinkRequest }) => {
      const { data, error } = await api.POST("/api/v1/releases/{id}/link", {
        params: { path: { id: args.releaseId } },
        body: args.body,
      });
      if (error)
        throw new Error(describeError(error, "failed to link release"));
      return data;
    },
    onSuccess: () => invalidateReleaseQueries(qc),
  });
}

export function useRejectRelease() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (releaseId: string) => {
      const { data, error } = await api.POST("/api/v1/releases/{id}/reject", {
        params: { path: { id: releaseId } },
      });
      if (error)
        throw new Error(describeError(error, "failed to reject release"));
      return data;
    },
    onSuccess: () => invalidateReleaseQueries(qc),
  });
}

export function useRetryRelease() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (releaseId: string) => {
      const { data, error } = await api.POST("/api/v1/releases/{id}/retry", {
        params: { path: { id: releaseId } },
      });
      if (error)
        throw new Error(describeError(error, "failed to retry release"));
      return data;
    },
    onSuccess: () => invalidateReleaseQueries(qc),
  });
}

export function useRetryAllReleases() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST("/api/v1/releases/retry-all", {});
      if (error)
        throw new Error(describeError(error, "failed to retry releases"));
      return data;
    },
    onSuccess: () => invalidateReleaseQueries(qc),
  });
}

function invalidateSourceQueries(qc: ReturnType<typeof useQueryClient>) {
  qc.invalidateQueries({ queryKey: ["sources"] });
  qc.invalidateQueries({ queryKey: ["stats"] });
}

function invalidateProviderQueries(qc: ReturnType<typeof useQueryClient>) {
  qc.invalidateQueries({ queryKey: ["providers"] });
  qc.invalidateQueries({ queryKey: ["stats"] });
}

export function usePollSource() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (name: string) => {
      const { data, error } = await api.POST("/api/v1/sources/{name}/poll", {
        params: { path: { name } },
      });
      if (error) throw new Error(describeError(error, "failed to poll source"));
      return data;
    },
    onSuccess: () => invalidateSourceQueries(qc),
  });
}

export function usePollAllSources() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST("/api/v1/sources/poll-all", {});
      if (error)
        throw new Error(describeError(error, "failed to poll sources"));
      return data;
    },
    onSuccess: () => invalidateSourceQueries(qc),
  });
}

export function useBackfillSource() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (args: { name: string; pages: number }) => {
      const { data, error } = await api.POST(
        "/api/v1/sources/{name}/backfill",
        {
          params: {
            path: { name: args.name },
            query: { pages: args.pages },
          },
        },
      );
      if (error)
        throw new Error(describeError(error, "failed to backfill source"));
      return data;
    },
    onSuccess: () => invalidateSourceQueries(qc),
  });
}

export function useRefreshProvider() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) => {
      const { data, error } = await api.POST(
        "/api/v1/providers/{id}/refresh-cache",
        {
          params: { path: { id } },
        },
      );
      if (error)
        throw new Error(describeError(error, "failed to refresh provider"));
      return data;
    },
    onSuccess: () => invalidateProviderQueries(qc),
  });
}

export function useRefreshAllProviders() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST(
        "/api/v1/providers/refresh-all",
        {},
      );
      if (error)
        throw new Error(describeError(error, "failed to refresh providers"));
      return data;
    },
    onSuccess: () => invalidateProviderQueries(qc),
  });
}
