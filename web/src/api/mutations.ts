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
