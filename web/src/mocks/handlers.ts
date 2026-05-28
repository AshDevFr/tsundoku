import { HttpResponse, http } from "msw";
import type { components } from "@/types/api.generated";

type SeriesListPage = components["schemas"]["SeriesListPage"];
type SeriesListItem = components["schemas"]["SeriesListItem"];
type SeriesDetail = components["schemas"]["SeriesDetail"];
type ReleasePage = components["schemas"]["ReleasePage"];
type ReleaseDto = components["schemas"]["ReleaseDto"];
type StatsResponse = components["schemas"]["StatsResponse"];
type UnresolvedRelease = components["schemas"]["UnresolvedRelease"];
type UnresolvedPage = components["schemas"]["UnresolvedPage"];
type LinkRequest = components["schemas"]["LinkRequest"];
type CreateSeriesRequest = components["schemas"]["CreateSeriesRequest"];
type BulkReviewRequest = components["schemas"]["BulkReviewRequest"];
type TagList = components["schemas"]["TagList"];

const NOW = Math.floor(Date.now() / 1000);
const ADMIN_TOKEN = "test-admin-token";

const SERIES: (SeriesListItem & {
  genres: string[];
  tags: string[];
  alternateTitles: string[];
})[] = [
  {
    id: 1,
    canonicalTitle: "Chainsaw Man",
    coverUrl: null,
    firstSeenAt: NOW - 86_400 * 30,
    lastReleaseAt: NOW - 3_600,
    kind: "manga",
    status: "ongoing",
    year: 2018,
    owned: false,
    description:
      "Denji has a simple dream: to live a happy and peaceful life. After his father's death he is left with a hefty debt to the yakuza. With his pet devil Pochita at his side he hunts devils to scrape by.",
    genres: ["action", "horror"],
    tags: ["devil hunter", "gore"],
    alternateTitles: ["チェンソーマン"],
    metadataSource: "offline_cache",
    releaseCount: 5,
  },
  {
    id: 2,
    canonicalTitle: "Re:Zero - Starting Life in Another World",
    coverUrl: null,
    firstSeenAt: NOW - 86_400 * 90,
    lastReleaseAt: NOW - 86_400,
    kind: "novel",
    status: "ongoing",
    year: 2014,
    owned: true,
    description:
      "Suddenly transported to a fantasy world, Subaru discovers that any time he dies he returns to a fixed save point.",
    genres: ["isekai", "drama"],
    tags: ["time loop"],
    alternateTitles: ["リゼロ"],
    metadataSource: "offline_cache",
    // Zero releases linked so the "Orphans only" filter has something
    // to surface in the mock UI.
    releaseCount: 0,
  },
  {
    id: 3,
    canonicalTitle: "Solo Leveling",
    coverUrl: null,
    firstSeenAt: NOW - 86_400 * 120,
    lastReleaseAt: NOW - 86_400 * 2,
    kind: "manhwa",
    status: "completed",
    year: 2018,
    owned: false,
    description:
      "The weakest hunter in the world levels up after a near-fatal dungeon raid and becomes the strongest.",
    genres: ["action", "fantasy"],
    tags: ["hunters", "leveling"],
    alternateTitles: [],
    metadataSource: "offline_cache",
    releaseCount: 3,
  },
];

// Mutable review queue so tests can assert that a release leaves the queue
// after a link / reject. Reset via `resetReviewQueue()`.
const INITIAL_QUEUE: UnresolvedRelease[] = [
  {
    id: "nyaa:9001",
    sourceKind: "nyaa",
    sourceName: "english-manga-trusted",
    externalId: "9001",
    title: "[Group] Mystery Series v01 (2024) (Digital) (CBZ)",
    link: "https://nyaa.si/view/9001",
    magnet: "magnet:?xt=urn:btih:dummy9001",
    torrentUrl: null,
    ddlUrl: null,
    infoHash: null,
    sizeBytes: 22_345_678,
    // Underscored names so they don't collide with the spaced "Mystery
    // Series v01" title regex tests use (Mantine Collapse keeps the file
    // list mounted even while visually collapsed).
    files: [
      "Mystery_Series_v01.cbz",
      "Mystery_Series_v02.cbz",
      "Mystery_Series_v03.cbz",
    ],
    formats: ["cbz"],
    postedAt: NOW - 1_800,
    observedAt: NOW - 1_200,
    seriesId: null,
    resolutionPath: null,
    resolutionConfidence: 0.6,
    resolutionStatus: "ambiguous",
    resolutionAttempts: 2,
    lastResolveAttemptAt: NOW - 1_200,
    candidates: [
      {
        seriesId: 1,
        seriesTitle: "Chainsaw Man",
        seriesCoverUrl: null,
        totalVolumes: 11,
        totalChapters: 97,
        kind: "manga",
        score: 0.72,
        reason: "fuzzy title (0.72)",
        provider: "mangabaka",
        externalId: "1",
        alternateTitles: ["チェンソーマン", "Chensō Man"],
      },
      {
        seriesId: 3,
        seriesTitle: "Solo Leveling",
        seriesCoverUrl: null,
        totalVolumes: null,
        totalChapters: 179,
        kind: "manhwa",
        score: 0.61,
        reason: "fuzzy title (0.61)",
        provider: "mangabaka",
        externalId: "3",
        alternateTitles: [],
      },
    ],
    searchQueries: ["Mystery Series"],
    cleanupRulesApplied: [
      "strip_brackets",
      "strip_parens",
      "strip_vol_compact",
    ],
    topCandidate: {
      seriesId: 1,
      seriesTitle: "Chainsaw Man",
      seriesCoverUrl: null,
      score: 0.72,
      reason: "fuzzy title (0.72)",
      provider: "mangabaka",
      externalId: "1",
      alternateTitles: ["チェンソーマン", "Chensō Man"],
    },
  },
  {
    id: "nyaa:9002",
    sourceKind: "nyaa",
    sourceName: "tsuna69",
    externalId: "9002",
    title: "[Uploader] Unknown Title v05",
    link: "https://nyaa.si/view/9002",
    magnet: null,
    torrentUrl: "https://nyaa.si/download/9002.torrent",
    ddlUrl: null,
    infoHash: null,
    sizeBytes: 100_000_000,
    files: ["unknown.cbz"],
    formats: ["cbz"],
    postedAt: NOW - 7_200,
    observedAt: NOW - 6_000,
    seriesId: null,
    resolutionPath: null,
    resolutionConfidence: null,
    resolutionStatus: "unresolved",
    resolutionAttempts: 1,
    lastResolveAttemptAt: NOW - 6_000,
    candidates: [],
    searchQueries: ["Unknown Title - A Story", "Unknown Title"],
    cleanupRulesApplied: [
      "strip_brackets",
      "strip_vol_compact",
      "split_subtitle",
    ],
    topCandidate: null,
  },
];

let queue: UnresolvedRelease[] = INITIAL_QUEUE.map((r) => ({
  ...r,
  candidates: r.candidates.map((c) => ({ ...c })),
}));

// Releases the operator marked `standalone`. The Kept browse view reads
// these via `GET /releases?status=standalone`; the `keep` handler appends.
const INITIAL_KEPT: ReleaseDto[] = [
  {
    id: "nyaa:7001",
    sourceKind: "nyaa",
    sourceName: "english-manga-trusted",
    externalId: "7001",
    title: "The Shonen Jump Guide to Making Manga (2022) (Digital) (LuCaZ)",
    link: "https://nyaa.si/view/7001",
    magnet: "magnet:?xt=urn:btih:dummy7001",
    torrentUrl: null,
    ddlUrl: null,
    infoHash: null,
    sizeBytes: 333_000_000,
    files: ["shonen_jump_guide_to_making_manga.cbz"],
    formats: ["cbz"],
    postedAt: NOW - 86_400,
    observedAt: NOW - 80_000,
    seriesId: null,
    resolutionPath: "standalone",
    resolutionConfidence: null,
    resolutionStatus: "standalone",
    resolutionAttempts: 2,
    lastResolveAttemptAt: NOW - 80_000,
    descriptionHtml:
      "# The Shonen Jump Guide\n\nAn **official guidebook**, not a series.",
    extractedLinks: {
      mangaupdates: "https://www.mangaupdates.com/series/zzz/x",
    },
  },
];

let kept: ReleaseDto[] = INITIAL_KEPT.map((r) => ({ ...r }));

export function resetReviewQueue() {
  queue = INITIAL_QUEUE.map((r) => ({
    ...r,
    candidates: r.candidates.map((c) => ({ ...c })),
  }));
  kept = INITIAL_KEPT.map((r) => ({ ...r }));
}

// Resolve a bulk request body into the queue rows it targets, mirroring the
// server: explicit `ids` win; otherwise the filter fields select the set
// (with the status clamp).
function bulkTargets(body: BulkReviewRequest): UnresolvedRelease[] {
  const QUEUE_STATUSES = ["unresolved", "ambiguous", "review_pending"];
  if (body.ids && body.ids.length > 0) {
    const ids = new Set(body.ids);
    return queue.filter((r) => ids.has(r.id));
  }
  const q = body.q?.trim().toLowerCase();
  return queue.filter((r) => {
    if (q && !r.title.toLowerCase().includes(q)) return false;
    if (body.sourceName && r.sourceName !== body.sourceName) return false;
    if (body.format && !r.formats.includes(body.format)) return false;
    if (body.status && QUEUE_STATUSES.includes(body.status)) {
      if (r.resolutionStatus !== body.status) return false;
    }
    return true;
  });
}

function requireAdmin(request: Request): Response | null {
  // Dev convenience: in mock mode any non-empty Bearer is accepted, so the
  // operator can paste whatever they have in `auth.admin_token` (or any
  // placeholder) without it needing to match a hard-coded constant. Tests
  // continue to set `ADMIN_TEST_TOKEN` explicitly via the auth store.
  const auth = request.headers.get("authorization") ?? "";
  if (/^Bearer\s+\S+/.test(auth)) return null;
  return new HttpResponse(
    JSON.stringify({ error: "unauthorized", message: "missing admin token" }),
    {
      status: 401,
      headers: { "content-type": "application/json" },
    },
  );
}

export const ADMIN_TEST_TOKEN = ADMIN_TOKEN;

export const handlers = [
  http.get("/api/v1/health", () => HttpResponse.json({ status: "ok" })),

  http.get("/api/v1/info", () =>
    HttpResponse.json({ name: "tsundoku", version: "1.0.1-mock" }),
  ),

  // SSE stream of job lifecycle events. The dev mock holds the
  // connection open with a single keepalive comment so the browser's
  // EventSource doesn't loop into reconnect attempts. No real events
  // are emitted; admin pages fall back to the normal poll cadence.
  http.get("/api/v1/events/jobs", () => {
    const stream = new ReadableStream({
      start(controller) {
        controller.enqueue(new TextEncoder().encode(": keepalive\n\n"));
      },
    });
    return new HttpResponse(stream, {
      headers: { "Content-Type": "text/event-stream" },
    });
  }),

  http.get("/api/v1/stats", () => {
    const stats: StatsResponse = {
      activeProvider: "mangabaka",
      series: SERIES.length,
      totalReleases: 12,
      releases: {
        resolved: 8,
        unresolved: queue.filter((r) => r.resolutionStatus === "unresolved")
          .length,
        ambiguous: queue.filter((r) => r.resolutionStatus === "ambiguous")
          .length,
        reviewPending: queue.filter(
          (r) => r.resolutionStatus === "review_pending",
        ).length,
        rejected: 0,
      },
    };
    return HttpResponse.json(stats);
  }),

  http.get("/api/v1/providers", () =>
    HttpResponse.json({
      items: [
        {
          id: "mangabaka",
          displayName: "MangaBaka",
          active: true,
          lastRefresh: {
            fetchedAt: NOW - 3_600,
            cacheVersion: "abc123",
            recordCount: 585_000,
            sourceUrl:
              "https://api.mangabaka.dev/v1/database/series.sqlite.tar.gz",
            bytesDownloaded: 476_000_000,
          },
          config: {
            apiFallback: true,
            apiKeySet: true,
            apiBaseUrl: "https://api.mangabaka.dev",
            offlineDumpUrl:
              "https://api.mangabaka.dev/v1/database/series.sqlite.tar.gz",
            offlineDumpConfigured: true,
            offlineCacheLoaded: true,
            offlineRefreshCron: "0 4 * * *",
            negativeCacheTtlDays: 7,
            timeoutSeconds: 60,
          },
        },
      ],
    }),
  ),

  http.get("/api/v1/providers/:id/search", ({ request, params }) => {
    const url = new URL(request.url);
    const q = url.searchParams.get("q") ?? "";
    const externalId = url.searchParams.get("externalId") ?? "";
    const providerId = String(params.id);
    if (providerId !== "mangabaka") {
      return new HttpResponse(
        JSON.stringify({
          error: "not_found",
          message: `provider ${providerId}`,
        }),
        { status: 404, headers: { "content-type": "application/json" } },
      );
    }
    if (!q.trim() && !externalId.trim()) {
      return new HttpResponse(
        JSON.stringify({
          error: "bad_request",
          message: "either q or externalId is required",
        }),
        { status: 400, headers: { "content-type": "application/json" } },
      );
    }
    // externalId path: one hit at score 1.0.
    if (externalId.trim()) {
      return HttpResponse.json({
        provider: providerId,
        hits: [
          {
            externalId: externalId.trim(),
            title: `Lookup #${externalId.trim()}`,
            year: null,
            coverUrl: null,
            kind: "manga",
            status: "ongoing",
            nativeTitle: null,
            genres: [],
            tags: [],
            score: 1.0,
          },
        ],
      });
    }
    // Title path: two canned hits.
    return HttpResponse.json({
      provider: providerId,
      hits: [
        {
          externalId: "mb-1",
          title: q.trim(),
          year: 2020,
          coverUrl: null,
          totalVolumes: 11,
          totalChapters: 97,
          kind: "manga",
          status: "ongoing",
          nativeTitle: null,
          genres: [],
          tags: [],
          score: 0.95,
        },
        {
          externalId: "mb-2",
          title: `${q.trim()} Side Stories`,
          year: 2021,
          coverUrl: null,
          kind: "manhwa",
          status: "completed",
          nativeTitle: null,
          genres: [],
          tags: [],
          score: 0.65,
        },
      ],
    });
  }),

  http.post("/api/v1/providers/refresh-all", ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    return HttpResponse.json({
      results: [{ provider: "mangabaka", triggered: true, skipped: false }],
    });
  }),

  http.post("/api/v1/providers/:id/refresh-cache", ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    return HttpResponse.json({
      provider: String(params.id),
      triggered: true,
      skipped: false,
    });
  }),

  http.get("/api/v1/sources", () =>
    HttpResponse.json({
      items: [
        {
          kind: "nyaa",
          name: "english-manga-trusted",
          lastPolledAt: NOW - 600,
          lastSuccessAt: NOW - 600,
          lastError: null,
          lastSummary: "75 new releases",
          config: {
            enabled: true,
            cron: "*/30 * * * *",
            feedUrl: "https://nyaa.si/?page=rss&c=3_1&f=2",
            fetchDetails: false,
            timeoutSeconds: 30,
            siteBaseUrl: "https://nyaa.si",
            maxPages: 1,
          },
        },
      ],
    }),
  ),

  http.post("/api/v1/sources/poll-all", ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    return HttpResponse.json({
      results: [
        {
          source: "english-manga-trusted",
          triggered: true,
          skipped: false,
        },
      ],
    });
  }),

  http.post("/api/v1/sources/:name/poll", ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    return HttpResponse.json({
      source: String(params.name),
      triggered: true,
      skipped: false,
    });
  }),

  http.post("/api/v1/sources/:name/backfill", ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const pages = Number(new URL(request.url).searchParams.get("pages") ?? 1);
    return HttpResponse.json({
      source: String(params.name),
      pages,
      triggered: true,
      skipped: false,
    });
  }),

  http.get("/api/v1/metrics/sources", () =>
    HttpResponse.json({
      items: [
        {
          sourceName: "english-manga-trusted",
          totalRuns: 12,
          successCount: 11,
          failureCount: 1,
          skippedCount: 0,
          fetchedSum: 250,
          newSum: 75,
          resolvedSum: 60,
          lastStartedAt: NOW - 600,
          lastStatus: "success",
          successRate: 11 / 12,
          outcomes: {
            knownId: 30,
            foreignId: 12,
            fuzzy: 18,
            review: 5,
            failed: 1,
          },
        },
      ],
      rangeSeconds: 24 * 3600,
      since: NOW - 24 * 3600,
      until: NOW,
    }),
  ),

  http.get("/api/v1/metrics/sources/:name", ({ params }) =>
    HttpResponse.json({
      sourceName: String(params.name),
      summary: {
        sourceName: String(params.name),
        totalRuns: 12,
        successCount: 11,
        failureCount: 1,
        skippedCount: 0,
        fetchedSum: 250,
        newSum: 75,
        resolvedSum: 60,
        lastStartedAt: NOW - 600,
        lastStatus: "success",
        successRate: 11 / 12,
        outcomes: {
          knownId: 30,
          foreignId: 12,
          fuzzy: 18,
          review: 5,
          failed: 1,
        },
      },
      buckets: [
        {
          bucketStart: NOW - 3600,
          successCount: 2,
          failureCount: 0,
          skippedCount: 0,
          fetchedSum: 24,
          newSum: 8,
        },
        {
          bucketStart: NOW - 1800,
          successCount: 1,
          failureCount: 1,
          skippedCount: 0,
          fetchedSum: 5,
          newSum: 1,
        },
      ],
      errorKinds: [{ kind: "network", count: 1 }],
      fetchLatency: { p50Ms: 1200, p95Ms: 4500, maxMs: 6000 },
      timeToResolution: { p50Seconds: 90, p95Seconds: 600, count: 60 },
      bucketSeconds: 3600,
      rangeSeconds: 24 * 3600,
      since: NOW - 24 * 3600,
      until: NOW,
    }),
  ),

  http.get("/api/v1/metrics/providers", () =>
    HttpResponse.json({
      items: [
        {
          providerId: "mangabaka",
          totalRuns: 4,
          successCount: 4,
          failureCount: 0,
          skippedCount: 0,
          bytesSum: 476_000_000,
          lastStartedAt: NOW - 3600,
          lastStatus: "success",
          successRate: 1.0,
        },
      ],
      rangeSeconds: 24 * 3600,
      since: NOW - 24 * 3600,
      until: NOW,
    }),
  ),

  http.get("/api/v1/metrics/review-queue", () =>
    HttpResponse.json({
      snapshots: [
        {
          capturedAt: NOW - 7200,
          pendingCount: 7,
          unresolvedCount: 3,
          ambiguousCount: 2,
          reviewPendingCount: 2,
          oldestPendingSeconds: 10_800,
        },
        {
          capturedAt: NOW - 3600,
          pendingCount: 5,
          unresolvedCount: 2,
          ambiguousCount: 1,
          reviewPendingCount: 2,
          oldestPendingSeconds: 9_000,
        },
      ],
      timeToDecisionP50Seconds: 240,
      closedCount: 18,
      rangeSeconds: 24 * 3600,
      since: NOW - 24 * 3600,
      until: NOW,
    }),
  ),

  http.get("/api/v1/metrics/providers/:id", ({ params }) =>
    HttpResponse.json({
      providerId: String(params.id),
      summary: {
        providerId: String(params.id),
        totalRuns: 4,
        successCount: 4,
        failureCount: 0,
        skippedCount: 0,
        bytesSum: 476_000_000,
        lastStartedAt: NOW - 3600,
        lastStatus: "success",
        successRate: 1.0,
      },
      buckets: [
        {
          bucketStart: NOW - 86400,
          successCount: 1,
          failureCount: 0,
          skippedCount: 0,
        },
        {
          bucketStart: NOW - 3600,
          successCount: 1,
          failureCount: 0,
          skippedCount: 0,
        },
      ],
      fetchLatency: { p50Ms: 320, p95Ms: 850, maxMs: 1100 },
      bucketSeconds: 3600,
      rangeSeconds: 24 * 3600,
      since: NOW - 24 * 3600,
      until: NOW,
    }),
  ),

  http.get("/api/v1/metrics/id-maps", () =>
    HttpResponse.json({
      externalIds: [
        { provider: "anilist", count: 12 },
        { provider: "mal", count: 24 },
        { provider: "mangaupdates", count: 38 },
      ],
      mangaupdatesRedirectCache: {
        modernCount: 17,
        tombstoneCount: 3,
        lastResolvedAt: NOW - 1800,
      },
    }),
  ),

  http.post("/api/v1/series", async ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const body = (await request.json()) as CreateSeriesRequest;
    const title = (body.canonicalTitle ?? "").trim();
    if (!title) {
      return new HttpResponse(
        JSON.stringify({
          error: "bad_request",
          message: "canonicalTitle must not be empty",
        }),
        { status: 400, headers: { "content-type": "application/json" } },
      );
    }
    const id = Math.max(0, ...SERIES.map((s) => s.id)) + 1;
    SERIES.push({
      id,
      canonicalTitle: title,
      coverUrl: body.coverUrl ?? null,
      firstSeenAt: NOW,
      lastReleaseAt: NOW,
      kind: body.kind ?? null,
      status: null,
      year: body.year ?? null,
      owned: false,
      description: body.description ?? null,
      genres: [],
      tags: [],
      alternateTitles: [],
      metadataSource: "manual",
      // Manually-created series start with no linked releases; the
      // operator may relink existing ones from the review queue.
      releaseCount: 0,
    });
    const detail: SeriesDetail = {
      id,
      canonicalTitle: title,
      alternateTitles: [],
      coverUrl: body.coverUrl ?? null,
      kind: body.kind ?? null,
      status: null,
      year: body.year ?? null,
      description: body.description ?? null,
      owned: false,
      genres: [],
      tags: [],
      externalIds: [],
      firstSeenAt: NOW,
      lastReleaseAt: NOW,
      metadataFetchedAt: NOW,
      metadataSource: "manual",
      highestVolume: null,
      highestChapter: null,
    };
    return HttpResponse.json(detail, { status: 201 });
  }),

  http.post("/api/v1/series/:id/refresh-metadata", ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const id = Number(params.id);
    const found = SERIES.find((s) => s.id === id);
    if (!found) return new HttpResponse(null, { status: 404 });
    const body: SeriesDetail = {
      id: found.id,
      canonicalTitle: found.canonicalTitle,
      alternateTitles: found.alternateTitles,
      coverUrl: found.coverUrl,
      kind: found.kind,
      status: found.status,
      year: found.year,
      description: found.description,
      owned: found.owned,
      genres: found.genres,
      tags: found.tags,
      externalIds: [
        {
          provider: "mangabaka",
          externalId: String(found.id * 1111),
          fetchedAt: NOW,
        },
      ],
      firstSeenAt: found.firstSeenAt,
      lastReleaseAt: found.lastReleaseAt,
      metadataFetchedAt: NOW,
      metadataSource: found.metadataSource,
      highestVolume: null,
      highestChapter: null,
    };
    return HttpResponse.json(body);
  }),

  http.get("/api/v1/series", ({ request }) => {
    const url = new URL(request.url);
    const kind = url.searchParams.get("kind");
    const status = url.searchParams.get("status");
    const owned = url.searchParams.get("owned");
    const hasReleases = url.searchParams.get("hasReleases");
    const genresCsv = url.searchParams.get("genres");
    const genresMode =
      url.searchParams.get("genresMode") === "all" ? "all" : "any";
    const tagsCsv = url.searchParams.get("tags");
    const tagsMode = url.searchParams.get("tagsMode") === "all" ? "all" : "any";
    const q = url.searchParams.get("q");
    const page = Number(url.searchParams.get("page") ?? "1");
    const pageSize = Number(url.searchParams.get("pageSize") ?? "24");

    let filtered = SERIES.slice();
    if (kind) filtered = filtered.filter((s) => s.kind === kind);
    if (status) filtered = filtered.filter((s) => s.status === status);
    if (owned === "true") filtered = filtered.filter((s) => s.owned === true);
    if (owned === "false") filtered = filtered.filter((s) => s.owned === false);
    if (hasReleases === "true")
      filtered = filtered.filter((s) => s.releaseCount > 0);
    if (hasReleases === "false")
      filtered = filtered.filter((s) => s.releaseCount === 0);
    const splitCsv = (s: string | null) =>
      s
        ?.split(",")
        .map((p) => p.trim().toLowerCase())
        .filter((p) => p.length > 0) ?? [];
    const genreNeedles = splitCsv(genresCsv);
    if (genreNeedles.length > 0) {
      filtered = filtered.filter((s) => {
        const owned = s.genres.map((g) => g.toLowerCase());
        return genresMode === "all"
          ? genreNeedles.every((n) => owned.includes(n))
          : genreNeedles.some((n) => owned.includes(n));
      });
    }
    const tagNeedles = splitCsv(tagsCsv);
    if (tagNeedles.length > 0) {
      filtered = filtered.filter((s) => {
        const owned = s.tags.map((t) => t.toLowerCase());
        return tagsMode === "all"
          ? tagNeedles.every((n) => owned.includes(n))
          : tagNeedles.some((n) => owned.includes(n));
      });
    }
    // Loose substring match against canonical + alternate titles. The
    // real backend reranks by Dice; for the mock we just need *some*
    // q-aware filtering so tests can assert it round-trips.
    if (q?.trim()) {
      const needle = q.trim().toLowerCase();
      filtered = filtered.filter(
        (s) =>
          s.canonicalTitle.toLowerCase().includes(needle) ||
          s.alternateTitles.some((t) => t.toLowerCase().includes(needle)),
      );
    }

    const start = (page - 1) * pageSize;
    const items = filtered
      .slice(start, start + pageSize)
      .map(({ alternateTitles: _a, ...rest }): SeriesListItem => rest);
    const body: SeriesListPage = {
      items,
      page,
      pageSize,
      total: filtered.length,
    };
    return HttpResponse.json(body);
  }),

  http.get("/api/v1/genres", () => {
    const counts = new Map<string, number>();
    for (const s of SERIES) {
      for (const g of s.genres) counts.set(g, (counts.get(g) ?? 0) + 1);
    }
    const body: TagList = {
      items: Array.from(counts.entries())
        .map(([name, seriesCount]) => ({ name, seriesCount }))
        .sort(
          (a, b) =>
            b.seriesCount - a.seriesCount || a.name.localeCompare(b.name),
        ),
    };
    return HttpResponse.json(body);
  }),

  http.get("/api/v1/tags", () => {
    const counts = new Map<string, number>();
    for (const s of SERIES) {
      for (const t of s.tags) counts.set(t, (counts.get(t) ?? 0) + 1);
    }
    const body: TagList = {
      items: Array.from(counts.entries())
        .map(([name, seriesCount]) => ({ name, seriesCount }))
        .sort(
          (a, b) =>
            b.seriesCount - a.seriesCount || a.name.localeCompare(b.name),
        ),
    };
    return HttpResponse.json(body);
  }),

  http.get("/api/v1/series/:id", ({ params }) => {
    const id = Number(params.id);
    const found = SERIES.find((s) => s.id === id);
    if (!found) return new HttpResponse(null, { status: 404 });
    const body: SeriesDetail = {
      id: found.id,
      canonicalTitle: found.canonicalTitle,
      alternateTitles: found.alternateTitles,
      coverUrl: found.coverUrl,
      kind: found.kind,
      status: found.status,
      year: found.year,
      description: found.description,
      owned: found.owned,
      genres: found.genres,
      tags: found.tags,
      externalIds: [
        {
          provider: "mangabaka",
          externalId: String(found.id * 1111),
          fetchedAt: NOW - 60,
        },
      ],
      firstSeenAt: found.firstSeenAt,
      lastReleaseAt: found.lastReleaseAt,
      metadataFetchedAt: NOW - 60,
      metadataSource: found.metadataSource,
      highestVolume: null,
      highestChapter: null,
    };
    return HttpResponse.json(body);
  }),

  http.get("/api/v1/releases", ({ request }) => {
    const url = new URL(request.url);
    const status = url.searchParams.get("status");
    if (status === "standalone") {
      const page = Number(url.searchParams.get("page") ?? "1");
      const pageSize = Number(url.searchParams.get("pageSize") ?? "20");
      const start = (page - 1) * pageSize;
      const items = kept.slice(start, start + pageSize);
      const body: ReleasePage = { items, page, pageSize, total: kept.length };
      return HttpResponse.json(body);
    }
    const seriesId = Number(url.searchParams.get("seriesId"));
    const all: ReleaseDto[] = [
      {
        id: "nyaa:111",
        sourceKind: "nyaa",
        sourceName: "english-manga-trusted",
        externalId: "111",
        title: `${SERIES[0]?.canonicalTitle} v01 (Digital) (CBZ)`,
        link: "https://nyaa.si/view/111",
        magnet: "magnet:?xt=urn:btih:dummy1",
        torrentUrl: null,
        ddlUrl: null,
        infoHash: null,
        sizeBytes: 12_345_678,
        files: ["chainsaw_man_v01.cbz"],
        formats: ["cbz"],
        postedAt: NOW - 7_200,
        observedAt: NOW - 6_000,
        seriesId: 1,
        resolutionPath: "fuzzy_title",
        resolutionConfidence: 0.92,
        resolutionStatus: "resolved",
        resolutionAttempts: 1,
        lastResolveAttemptAt: NOW - 6_000,
      },
    ];
    const items = Number.isFinite(seriesId)
      ? all.filter((r) => r.seriesId === seriesId)
      : all;
    const body: ReleasePage = {
      items,
      page: 1,
      pageSize: 50,
      total: items.length,
    };
    return HttpResponse.json(body);
  }),

  http.get("/api/v1/releases/unresolved", ({ request }) => {
    const url = new URL(request.url);
    const page = Number(url.searchParams.get("page") ?? "1");
    const pageSize = Number(url.searchParams.get("pageSize") ?? "20");
    const q = url.searchParams.get("q")?.trim().toLowerCase();
    const sourceName = url.searchParams.get("sourceName");
    const format = url.searchParams.get("format");
    const status = url.searchParams.get("status");
    const QUEUE_STATUSES = ["unresolved", "ambiguous", "review_pending"];
    const filtered = queue.filter((r) => {
      if (q && !r.title.toLowerCase().includes(q)) return false;
      if (sourceName && r.sourceName !== sourceName) return false;
      if (format && !r.formats.includes(format)) return false;
      // Mirror the server clamp: an out-of-queue status is ignored.
      if (status && QUEUE_STATUSES.includes(status)) {
        if (r.resolutionStatus !== status) return false;
      }
      return true;
    });
    const start = (page - 1) * pageSize;
    const items = filtered.slice(start, start + pageSize);
    const body: UnresolvedPage = {
      items,
      page,
      pageSize,
      total: filtered.length,
    };
    return HttpResponse.json(body);
  }),

  // Registered before the `:id` POST handlers below: MSW is first-match-wins,
  // so `/releases/:id/reject` would otherwise swallow `/releases/bulk/reject`
  // with id="bulk". (axum prioritizes the static segment, so the server is
  // fine — this ordering only matters for the mock.)
  http.post("/api/v1/releases/bulk/reject", async ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const body = (await request.json()) as BulkReviewRequest;
    const targets = bulkTargets(body);
    const ids = new Set(targets.map((r) => r.id));
    queue = queue.filter((r) => !ids.has(r.id));
    return HttpResponse.json({ rejected: targets.length });
  }),

  http.post("/api/v1/releases/bulk/retry", async ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const body = (await request.json()) as BulkReviewRequest;
    const matched = bulkTargets(body).length;
    // The mock doesn't re-resolve; rows stay in the queue (as they would
    // until the background batch reclassifies them).
    return HttpResponse.json({
      triggered: matched > 0,
      skipped: false,
      matched,
    });
  }),

  http.post("/api/v1/releases/:id/link", async ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const id = String(params.id);
    const idx = queue.findIndex((r) => r.id === id);
    const release = idx >= 0 ? queue[idx] : null;
    if (!release)
      return new HttpResponse(
        JSON.stringify({ error: "not_found", message: `release ${id}` }),
        { status: 404, headers: { "content-type": "application/json" } },
      );
    const body = (await request.json()) as LinkRequest;
    let seriesId = body.seriesId ?? null;
    if (!seriesId && body.provider && body.externalId) {
      // Pretend the provider resolved to series #1 for the test fixture.
      seriesId = 1;
    }
    if (!seriesId)
      return new HttpResponse(
        JSON.stringify({
          error: "bad_request",
          message: "missing seriesId or provider+externalId",
        }),
        { status: 400, headers: { "content-type": "application/json" } },
      );
    queue.splice(idx, 1);
    const updated: ReleaseDto = {
      ...release,
      seriesId,
      resolutionStatus: "resolved",
      resolutionPath: "manual",
      resolutionConfidence: 1,
      resolutionAttempts: release.resolutionAttempts + 1,
      lastResolveAttemptAt: NOW,
    };
    return HttpResponse.json(updated);
  }),

  http.post("/api/v1/releases/:id/reject", ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const id = String(params.id);
    const idx = queue.findIndex((r) => r.id === id);
    const release = idx >= 0 ? queue[idx] : undefined;
    if (!release)
      return new HttpResponse(
        JSON.stringify({ error: "not_found", message: `release ${id}` }),
        { status: 404, headers: { "content-type": "application/json" } },
      );
    queue.splice(idx, 1);
    const updated: ReleaseDto = {
      ...release,
      resolutionStatus: "rejected",
      resolutionPath: "rejected",
      resolutionConfidence: null,
      resolutionAttempts: release.resolutionAttempts + 1,
      lastResolveAttemptAt: NOW,
    };
    return HttpResponse.json(updated);
  }),

  http.post("/api/v1/releases/:id/keep", ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const id = String(params.id);
    const idx = queue.findIndex((r) => r.id === id);
    const release = idx >= 0 ? queue[idx] : undefined;
    if (!release)
      return new HttpResponse(
        JSON.stringify({ error: "not_found", message: `release ${id}` }),
        { status: 404, headers: { "content-type": "application/json" } },
      );
    queue.splice(idx, 1);
    const updated: ReleaseDto = {
      ...release,
      seriesId: null,
      resolutionStatus: "standalone",
      resolutionPath: "standalone",
      resolutionConfidence: null,
      resolutionAttempts: release.resolutionAttempts + 1,
      lastResolveAttemptAt: NOW,
    };
    kept.unshift(updated);
    return HttpResponse.json(updated);
  }),

  http.post("/api/v1/releases/:id/retry", ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const id = String(params.id);
    const queueIdx = queue.findIndex((r) => r.id === id);
    if (queueIdx >= 0) {
      const release = queue[queueIdx];
      const updated: ReleaseDto = {
        ...release,
        resolutionAttempts: release.resolutionAttempts + 1,
        lastResolveAttemptAt: NOW,
      };
      queue[queueIdx] = { ...release, ...updated };
      return HttpResponse.json(updated);
    }
    // A kept (standalone) release can be pulled back into the pipeline.
    const keptIdx = kept.findIndex((r) => r.id === id);
    if (keptIdx >= 0) {
      const release = kept[keptIdx];
      const updated: ReleaseDto = {
        ...release,
        resolutionAttempts: release.resolutionAttempts + 1,
        lastResolveAttemptAt: NOW,
      };
      kept[keptIdx] = { ...release, ...updated };
      return HttpResponse.json(updated);
    }
    return new HttpResponse(
      JSON.stringify({ error: "not_found", message: `release ${id}` }),
      { status: 404, headers: { "content-type": "application/json" } },
    );
  }),
];
