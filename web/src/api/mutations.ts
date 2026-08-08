import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { components } from "@/types/api.generated";

type LinkRequest = components["schemas"]["LinkRequest"];
type CreateSeriesRequest = components["schemas"]["CreateSeriesRequest"];
type UpdateSeriesRequest = components["schemas"]["UpdateSeriesRequest"];
type BulkReviewRequest = components["schemas"]["BulkReviewRequest"];
type BulkLinkRequest = components["schemas"]["BulkLinkRequest"];
type SendToClientRequest = components["schemas"]["SendToClientRequest"];
type CreateSeriesFromProviderRequest =
  components["schemas"]["CreateSeriesFromProviderRequest"];
type BulkWishlistRequest = components["schemas"]["BulkWishlistRequest"];
type BulkRefreshMetadataRequest =
  components["schemas"]["BulkRefreshMetadataRequest"];
type BulkSearchReleasesRequest =
  components["schemas"]["BulkSearchReleasesRequest"];

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
  // The grouping panel's clusters are a separate query key (per-element match,
  // so the line above doesn't cover it); refresh them too or a decision leaves
  // a now-stale group in the list until a manual refetch.
  qc.invalidateQueries({ queryKey: ["releases-unresolved-groups"] });
  qc.invalidateQueries({ queryKey: ["releases-kept"] });
  qc.invalidateQueries({ queryKey: ["stats"] });
  qc.invalidateQueries({ queryKey: ["series-list"] });
  // A relink moves a release between series; refresh both the per-series
  // release lists and the detail headers so the source series drops it and
  // the target picks it up without a manual reload.
  qc.invalidateQueries({ queryKey: ["series-releases"] });
  qc.invalidateQueries({ queryKey: ["series-detail"] });
}

/// Trigger a Codex presence sweep. Invalidates the status row and the series
/// caches so freshly-linked ownership shows up without a manual reload.
export function useCodexRefresh() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST("/api/v1/codex/refresh");
      if (error)
        throw new Error(
          describeError(error, "failed to trigger codex refresh"),
        );
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["codex-status"] });
      qc.invalidateQueries({ queryKey: ["series-list"] });
      qc.invalidateQueries({ queryKey: ["series-detail"] });
    },
  });
}

/// Run an on-demand Codex `/info` preflight. A failed probe still resolves
/// (200 with `reachable: false`); invalidates the status row so the card and
/// its history refresh.
export function useTestCodex() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST("/api/v1/codex/test");
      if (error)
        throw new Error(
          describeError(error, "failed to test codex connection"),
        );
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["codex-status"] });
    },
  });
}

/// Run an on-demand download-client connection test. A failed probe still
/// resolves (200 with `reachable: false`); invalidates the status query so the
/// Download page reflects the result and any new history row.
export function useTestDownload() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST("/api/v1/download/test");
      if (error)
        throw new Error(describeError(error, "failed to test download client"));
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["download-status"] });
    },
  });
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

/// Push a discovered release into the configured torrent client. `body` is the
/// optional per-send override (`{}` ⇒ config defaults, the one-click path).
/// Invalidates the release views so the "Sent" badge appears without a refetch.
export function useSendToClient() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (args: {
      releaseId: string;
      body: SendToClientRequest;
    }) => {
      const { data, error } = await api.POST(
        "/api/v1/releases/{id}/send-to-client",
        {
          params: { path: { id: args.releaseId } },
          body: args.body,
        },
      );
      if (error)
        throw new Error(
          describeError(error, "failed to send to torrent client"),
        );
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

/// Edit a manual series' descriptive fields. Backend rejects provider-backed
/// rows with 409, so the caller only exposes this for `metadataSource ===
/// "manual"`. Invalidates the detail + list caches on success.
export function useUpdateSeries() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (args: { id: number; body: UpdateSeriesRequest }) => {
      const { data, error } = await api.PATCH("/api/v1/series/{id}", {
        params: { path: { id: args.id } },
        body: args.body,
      });
      if (error)
        throw new Error(describeError(error, "failed to update series"));
      return data;
    },
    onSuccess: (_data, { id }) => {
      qc.invalidateQueries({ queryKey: ["series-detail", id] });
      qc.invalidateQueries({ queryKey: ["series-list"] });
    },
  });
}

/// Toggle a series' `ignore_completion` flag. When on, the series' Codex status
/// is forced to `ignored`, muting the perpetually-false "behind" signal for
/// series read in omnibus. Unlike [`useUpdateSeries`], the backend accepts
/// provider-backed rows here, so this is offered for any owned series.
/// Invalidates the detail + list caches so the badge and any `codexStatus`
/// filter update without a reload.
export function useSetIgnoreCompletion() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (args: { id: number; ignore: boolean }) => {
      const { data, error } = await api.PUT(
        "/api/v1/series/{id}/ignore-completion",
        {
          params: { path: { id: args.id } },
          body: { ignore: args.ignore },
        },
      );
      if (error)
        throw new Error(
          describeError(error, "failed to update completion tracking"),
        );
      return data;
    },
    onSuccess: (_data, { id }) => {
      qc.invalidateQueries({ queryKey: ["series-detail", id] });
      qc.invalidateQueries({ queryKey: ["series-list"] });
    },
  });
}

/// Clip or un-clip a series from the operator's wishlist (a curated "download
/// later" list). Works on any series, provider-backed or manual; independent of
/// Codex ownership. Invalidates the detail + list caches so the card star, the
/// detail button, and any `wishlisted` filter update without a reload.
export function useSetWishlisted() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (args: { id: number; wishlisted: boolean }) => {
      const { data, error } = await api.PUT("/api/v1/series/{id}/wishlist", {
        params: { path: { id: args.id } },
        body: { wishlisted: args.wishlisted },
      });
      if (error)
        throw new Error(describeError(error, "failed to update wishlist"));
      return data;
    },
    onSuccess: (_data, { id }) => {
      qc.invalidateQueries({ queryKey: ["series-detail", id] });
      qc.invalidateQueries({ queryKey: ["series-list"] });
    },
  });
}

/// Clip or un-clip a whole selection of series in one request (`wishlisted`
/// is an explicit set, not a per-row toggle, so mixed selections converge).
/// Same cache invalidation as the single-series [`useSetWishlisted`].
export function useBulkSetWishlisted() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (body: BulkWishlistRequest) => {
      const { data, error } = await api.PUT("/api/v1/series/bulk/wishlist", {
        body,
      });
      if (error || !data)
        throw new Error(describeError(error, "failed to update wishlist"));
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["series-detail"] });
      qc.invalidateQueries({ queryKey: ["series-list"] });
    },
  });
}

/// Refresh a whole selection of series from the active provider. Synchronous
/// on the backend (offline-dump reads); the response reports per-id skips
/// (`skipped`) alongside the `refreshed` count — surface both to the operator.
export function useBulkRefreshMetadata() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (body: BulkRefreshMetadataRequest) => {
      const { data, error } = await api.POST(
        "/api/v1/series/bulk/refresh-metadata",
        { body },
      );
      if (error || !data)
        throw new Error(describeError(error, "failed to refresh series"));
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["series-detail"] });
      qc.invalidateQueries({ queryKey: ["series-list"] });
    },
  });
}

/// Launch release searches for a whole selection in one dispatched batch
/// (sequential walks behind the entry's lock). `skipped: true` in the
/// response means a walk was already in flight and nothing ran — callers
/// should tell the operator rather than treat it as success. Invalidates the
/// run-history root so every affected series' timeline picks up its new row.
export function useBulkSearchReleases() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (body: BulkSearchReleasesRequest) => {
      const { data, error } = await api.POST(
        "/api/v1/series/bulk/search-releases",
        { body },
      );
      if (error || !data)
        throw new Error(describeError(error, "failed to start searches"));
      return data;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["search-runs"] }),
  });
}

/// Materialize a series from a metadata provider (the "add from MangaBaka"
/// flow), optionally clipping it to the wishlist. Idempotent server-side on
/// `(provider, externalId)`. Invalidates the series list (the wishlist page
/// reads it) and stats so the new row shows without a reload.
export function useCreateSeriesFromProvider() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (body: CreateSeriesFromProviderRequest) => {
      const { data, error } = await api.POST("/api/v1/series/from-provider", {
        body,
      });
      if (error) throw new Error(describeError(error, "failed to add series"));
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

/// Link a set of review-queue releases to a single series in one request.
/// The body carries an explicit `ids` list plus the series target (either a
/// `seriesId` or a `provider` + `externalId` pair), mirroring the
/// single-release link. Used by the bulk "assign to series" / "create &
/// link all" flows after selecting several releases of the same series.
export function useBulkLink() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (body: BulkLinkRequest) => {
      const { data, error } = await api.POST("/api/v1/releases/bulk/link", {
        body,
      });
      if (error)
        throw new Error(describeError(error, "failed to link releases"));
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

/// Launch a per-series release search. `search` omitted ⇒ the default
/// entry. Invalidates the series' run list so the new `running` row shows
/// up (and starts the poll cycle) immediately.
export function useSearchReleases() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (args: { seriesId: number; search?: string }) => {
      const { data, error } = await api.POST(
        "/api/v1/series/{id}/search-releases",
        {
          params: { path: { id: args.seriesId } },
          body: { search: args.search ?? null },
        },
      );
      if (error || !data)
        throw new Error(describeError(error, "failed to start search"));
      return data;
    },
    onSuccess: (_data, args) =>
      qc.invalidateQueries({ queryKey: ["search-runs", args.seriesId] }),
  });
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

export function useReenrichReleases() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (args: {
      statuses: string[];
      onlyMissingDetails: boolean;
      /** Omitted = every origin (sources, searches, removed origins). */
      sources?: string[];
    }) => {
      const { data, error } = await api.POST("/api/v1/releases/re-enrich", {
        body: {
          statuses: args.statuses,
          onlyMissingDetails: args.onlyMissingDetails,
          sources: args.sources ?? null,
        },
      });
      if (error)
        throw new Error(describeError(error, "failed to re-enrich releases"));
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
/// Delete the orphan series the dry run listed. Irreversible; the caller is
/// responsible for confirming first.
export function usePurgeOrphanSeries() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (excludeWishlisted: boolean) => {
      const { data, error } = await api.POST(
        "/api/v1/maintenance/orphan-series/purge",
        { body: { excludeWishlisted } },
      );
      if (error)
        throw new Error(describeError(error, "failed to purge orphan series"));
      return data;
    },
    onSuccess: () => {
      // The dry run and every series listing just changed.
      qc.invalidateQueries({ queryKey: ["orphan-series"] });
      qc.invalidateQueries({ queryKey: ["series"] });
      qc.invalidateQueries({ queryKey: ["stats"] });
    },
  });
}

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

/// Manually trigger a series-metadata refresh against the active provider.
/// Same locking semantics as the scheduled job: an in-flight tick causes
/// the request to no-op with `triggered: false, skipped: true`.
///
/// Pass `all: false` (the default) for a single settings-bounded tick
/// (honors `batch_size` + `min_age_days`), or `all: true` to drain every
/// eligible row in repeated batches, ignoring the min-age floor. The
/// pending mutation's `variables` carries the `all` flag so the caller can
/// show per-button loading state. Paired with [`useInvalidateMetadataHashes`]
/// from the Maintenance page so the operator can clear hashes and rewrite
/// rows without waiting for the next cron tick.
export function useRefreshAllSeries() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (all: boolean) => {
      const { data, error } = await api.POST("/api/v1/series/refresh-all", {
        params: { query: { all } },
      });
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

/// Recompute every release's volume/chapter span and every series'
/// `highestVolume` / `highestChapter` from the stored file lists (titles as
/// fallback). Network-free and idempotent; authoritative (a series' marks
/// are replaced with the MAX across its linked releases, so values can also
/// go down after a parsing-strategy change). Use it to backfill a catalog
/// that predates span detection, or after the parser changes.
export function useRecomputeSpans() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const { data, error } = await api.POST(
        "/api/v1/series/recompute-spans",
        {},
      );
      if (error)
        throw new Error(describeError(error, "failed to recompute spans"));
      return data;
    },
    onSuccess: () => {
      // Both list badges and detail page read the recomputed marks.
      qc.invalidateQueries({ queryKey: ["series-list"] });
      qc.invalidateQueries({ queryKey: ["series-detail"] });
    },
  });
}

/// Add a single release by pasting its post URL, for something the polled
/// feeds never surfaced. Resolves synchronously, so the caller gets the
/// outcome (and whether the catalog already held it) in the response.
export function useImportRelease() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (url: string) => {
      const { data, error } = await api.POST("/api/v1/releases/import", {
        body: { url },
      });
      if (error)
        throw new Error(describeError(error, "failed to import release"));
      return data;
    },
    onSuccess: () => invalidateReleaseQueries(qc),
  });
}
