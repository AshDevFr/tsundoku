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
    files: ["mystery_series_v01.cbz"],
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
    searchQueries: ["Unknown Title"],
    cleanupRulesApplied: ["strip_brackets", "strip_vol_compact"],
    topCandidate: null,
  },
];

let queue: UnresolvedRelease[] = INITIAL_QUEUE.map((r) => ({
  ...r,
  candidates: r.candidates.map((c) => ({ ...c })),
}));

export function resetReviewQueue() {
  queue = INITIAL_QUEUE.map((r) => ({
    ...r,
    candidates: r.candidates.map((c) => ({ ...c })),
  }));
}

function requireAdmin(request: Request): Response | null {
  const auth = request.headers.get("authorization");
  if (auth === `Bearer ${ADMIN_TOKEN}`) return null;
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

  http.get("/api/v1/series", ({ request }) => {
    const url = new URL(request.url);
    const kind = url.searchParams.get("kind");
    const status = url.searchParams.get("status");
    const owned = url.searchParams.get("owned");
    const genre = url.searchParams.get("genre");
    const tag = url.searchParams.get("tag");
    const q = url.searchParams.get("q");
    const page = Number(url.searchParams.get("page") ?? "1");
    const pageSize = Number(url.searchParams.get("pageSize") ?? "24");

    let filtered = SERIES.slice();
    if (kind) filtered = filtered.filter((s) => s.kind === kind);
    if (status) filtered = filtered.filter((s) => s.status === status);
    if (owned === "true") filtered = filtered.filter((s) => s.owned === true);
    if (owned === "false") filtered = filtered.filter((s) => s.owned === false);
    if (genre) {
      const needle = genre.toLowerCase();
      filtered = filtered.filter((s) =>
        s.genres.some((g) => g.toLowerCase() === needle),
      );
    }
    if (tag) {
      const needle = tag.toLowerCase();
      filtered = filtered.filter((s) =>
        s.tags.some((t) => t.toLowerCase() === needle),
      );
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
      metadataSource: "offline_cache",
      highestVolume: null,
      highestChapter: null,
    };
    return HttpResponse.json(body);
  }),

  http.get("/api/v1/releases", ({ request }) => {
    const url = new URL(request.url);
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
    const start = (page - 1) * pageSize;
    const items = queue.slice(start, start + pageSize);
    const body: UnresolvedPage = {
      items,
      page,
      pageSize,
      total: queue.length,
    };
    return HttpResponse.json(body);
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

  http.post("/api/v1/releases/:id/retry", ({ request, params }) => {
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
    const updated: ReleaseDto = {
      ...release,
      resolutionAttempts: release.resolutionAttempts + 1,
      lastResolveAttemptAt: NOW,
    };
    queue[idx] = { ...release, ...updated };
    return HttpResponse.json(updated);
  }),
];
