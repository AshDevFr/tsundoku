import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { components } from "@/types/api.generated";

type LinkRequest = components["schemas"]["LinkRequest"];
type CreateSeriesRequest = components["schemas"]["CreateSeriesRequest"];
type BulkReviewRequest = components["schemas"]["BulkReviewRequest"];

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
  qc.invalidateQueries({ queryKey: ["releases-kept"] });
  qc.invalidateQueries({ queryKey: ["stats"] });
  qc.invalidateQueries({ queryKey: ["series-list"] });
  // A relink moves a release between series; refresh both the per-series
  // release lists and the detail headers so the source series drops it and
  // the target picks it up without a manual reload.
  qc.invalidateQueries({ queryKey: ["series-releases"] });
  qc.invalidateQueries({ queryKey: ["series-detail"] });
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

/// Create a manual (provider-less) series. Used from the review queue when
/// MangaBaka lacks a series the operator wants to link a release to. The
/// caller then links the release via `useLinkRelease({ seriesId })`.
export function useCreateSeries() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (body: CreateSeriesRequest) => {
      const { data, error } = await api.POST("/api/v1/series", { body });
      if (error)
        throw new Error(describeError(error, "failed to create series"));
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["series-list"] });
      qc.invalidateQueries({ queryKey: ["stats"] });
    },
  });
}

export function useKeepRelease() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (releaseId: string) => {
      const { data, error } = await api.POST("/api/v1/releases/{id}/keep", {
        params: { path: { id: releaseId } },
      });
      if (error)
        throw new Error(describeError(error, "failed to keep release"));
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

/// Reject a set of review-queue releases in one request. The body carries
/// either an explicit `ids` list or the filter fields ("all matching"); see
/// `BulkReviewRequest`.
export function useBulkReject() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (body: BulkReviewRequest) => {
      const { data, error } = await api.POST("/api/v1/releases/bulk/reject", {
        body,
      });
      if (error)
        throw new Error(describeError(error, "failed to reject releases"));
      return data;
    },
    onSuccess: () => invalidateReleaseQueries(qc),
  });
}

/// Retry a set of review-queue releases as a background batch. Same body
/// shape as `useBulkReject`; the response reports `triggered`/`skipped`/
/// `matched`.
export function useBulkRetry() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (body: BulkReviewRequest) => {
      const { data, error } = await api.POST("/api/v1/releases/bulk/retry", {
        body,
      });
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

export function useReenrichSource() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (args: { name: string; statuses: string[] }) => {
      const { data, error } = await api.POST(
        "/api/v1/sources/{name}/re-enrich",
        {
          params: { path: { name: args.name } },
          body: { statuses: args.statuses },
        },
      );
      if (error)
        throw new Error(describeError(error, "failed to re-enrich source"));
      return data;
    },
    // Re-enrich refreshes release detail columns in place; bust the release
    // views (and stats) so the freshly-pulled fields show on next read.
    onSuccess: () => {
      invalidateSourceQueries(qc);
      invalidateReleaseQueries(qc);
    },
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

/// Clear `metadata_hash` for every provider-backed series row in scope.
/// The persist layer short-circuits the series UPDATE when the incoming
/// provider payload hashes to the stored value; that's the right call
/// for steady-state refreshes, but it strands existing rows whenever a
/// new denormalized column lands on the `series` table (upstream payload
/// unchanged → hash matches → write skipped → new column stays NULL
/// forever). This mutation is the operator escape hatch for that
/// scenario. Pair it with [`useRefreshSeriesMetadata`] or the
/// series-refresh cron to actually rewrite the rows.
///
/// Manual rows (`metadata_source = 'manual'`) are always left alone;
/// the response carries `skippedManual` for operator transparency.
export function useInvalidateMetadataHashes() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (args: { provider?: string } = {}) => {
      const { data, error } = await api.POST(
        "/api/v1/series/invalidate-metadata-hashes",
        {
          params: {
            query: args.provider ? { provider: args.provider } : {},
          },
        },
      );
      if (error)
        throw new Error(
          describeError(error, "failed to invalidate metadata hashes"),
        );
      return data;
    },
    onSuccess: () => {
      // Series rows now have NULL hashes but identical user-visible
      // fields, so list/detail queries don't need invalidation. The
      // visible change lands when the next refresh tick rewrites the
      // rows; that flow already invalidates these query keys.
      qc.invalidateQueries({ queryKey: ["stats"] });
    },
  });
}

export function useRefreshSeriesMetadata() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (seriesId: number) => {
      const { data, error } = await api.POST(
        "/api/v1/series/{id}/refresh-metadata",
        { params: { path: { id: seriesId } } },
      );
      if (error)
        throw new Error(describeError(error, "failed to refresh series"));
      return data;
    },
    onSuccess: (_data, seriesId) => {
      qc.invalidateQueries({ queryKey: ["series-detail", seriesId] });
      qc.invalidateQueries({ queryKey: ["series-list"] });
    },
  });
}

/// Wipe the on-disk cover-image cache served by `/api/v1/covers/*`.
/// Files come back into existence on demand as the UI requests covers
/// again, so the cost is bandwidth, not correctness. Use this when an
/// upstream cover was corrected and you want the proxy to pull the new
/// bytes immediately instead of waiting for the URL to rotate.
export function useInvalidateCoverCache() {
  return useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST(
        "/api/v1/covers/invalidate-cache",
        {},
      );
      if (error)
        throw new Error(
          describeError(error, "failed to invalidate cover cache"),
        );
      return data;
    },
    // No query invalidation: the proxy URLs the UI renders haven't
    // changed (still `/api/v1/covers/{id}`); only the on-disk cache
    // entries did. The next render uses the browser cache until its
    // own TTL expires; a hard refresh forces fresh proxy hits.
  });
}

/// Manually trigger a series-metadata refresh tick against the active
/// provider. Same locking semantics as the scheduled job: an in-flight
/// tick causes the request to no-op with `triggered: false, skipped: true`.
/// Paired with [`useInvalidateMetadataHashes`] from the Maintenance page
/// so the operator can clear hashes and immediately rewrite the rows
/// instead of waiting for the next cron tick.
export function useRefreshAllSeries() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST("/api/v1/series/refresh-all", {});
      if (error)
        throw new Error(
          describeError(error, "failed to refresh series metadata"),
        );
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["series-list"] });
      qc.invalidateQueries({ queryKey: ["series-detail"] });
      qc.invalidateQueries({ queryKey: ["stats"] });
    },
  });
}
