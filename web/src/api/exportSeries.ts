// Series catalog export download helper.
//
// The export endpoint (`GET /api/v1/series/export`) is admin-only and returns
// a file attachment, not JSON — so it doesn't fit the typed openapi-fetch
// client. We build the URL by hand, attach the admin bearer, and turn the
// response into a browser download. `buildExportUrl` is split out as a pure
// function so the URL composition is unit-testable without touching the DOM.

import { currentAdminToken } from "@/stores/auth";

export type ExportFormat = "json" | "csv" | "markdown";

/// Structured filters mirrored from the series-list endpoint. Empty / absent
/// values are omitted from the query, matching the backend's lenient "no
/// constraint" handling. The relevance `q` search is intentionally absent: it
/// is a ranking path, not a catalog filter (see Phase 1 notes).
export interface ExportFilters {
  kind?: string | null;
  status?: string | null;
  metadataSource?: string | null;
  hasReleases?: boolean | null;
  codexStatus?: string[];
  genres?: string[];
  tags?: string[];
}

export interface ExportOptions {
  format: ExportFormat;
  /// Selected field keys (camelCase, matching the backend `ExportField`
  /// catalog). `canonicalTitle` is always honored server-side regardless.
  fields: string[];
  includeReleases: boolean;
  filters: ExportFilters;
}

/// Compose the `/api/v1/series/export` request path + query string from the
/// export options. Pure (no DOM, no fetch) so it can be asserted directly.
export function buildExportUrl(opts: ExportOptions): string {
  const params = new URLSearchParams();
  params.set("format", opts.format);
  if (opts.fields.length > 0) {
    params.set("fields", opts.fields.join(","));
  }
  if (opts.includeReleases) {
    params.set("includeReleases", "true");
  }

  const f = opts.filters;
  if (f.kind) params.set("kind", f.kind);
  if (f.status) params.set("status", f.status);
  if (f.metadataSource) params.set("metadataSource", f.metadataSource);
  if (f.hasReleases != null) params.set("hasReleases", String(f.hasReleases));
  if (f.codexStatus && f.codexStatus.length > 0) {
    params.set("codexStatus", f.codexStatus.join(","));
  }
  if (f.genres && f.genres.length > 0) params.set("genres", f.genres.join(","));
  if (f.tags && f.tags.length > 0) params.set("tags", f.tags.join(","));

  return `/api/v1/series/export?${params.toString()}`;
}

/// Pull the filename out of a `Content-Disposition` header, falling back to a
/// date-stamped default when the header is absent or unparseable.
function filenameFrom(
  disposition: string | null,
  format: ExportFormat,
): string {
  const match = disposition?.match(/filename="?([^"]+)"?/i);
  if (match?.[1]) return match[1];
  const ext = format === "markdown" ? "md" : format;
  return `tsundoku-series-export.${ext}`;
}

/// Save a blob to disk via a transient object-URL anchor. No-ops gracefully in
/// environments without `URL.createObjectURL` (some test runners).
function saveBlob(blob: Blob, filename: string): void {
  if (typeof URL.createObjectURL !== "function") return;
  const href = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = href;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(href);
}

/// Fetch the export with the admin bearer and trigger a browser download.
/// Throws on a non-2xx response so the caller can surface an error.
export async function downloadSeriesExport(opts: ExportOptions): Promise<void> {
  const token = currentAdminToken();
  const origin =
    typeof window !== "undefined" ? window.location.origin : "http://localhost";
  const url = new URL(buildExportUrl(opts), origin);

  const res = await fetch(url, {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
  });
  if (!res.ok) {
    throw new Error(`Export failed (${res.status})`);
  }
  const blob = await res.blob();
  const filename = filenameFrom(
    res.headers.get("content-disposition"),
    opts.format,
  );
  saveBlob(blob, filename);
}
