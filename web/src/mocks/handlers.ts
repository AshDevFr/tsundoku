import { HttpResponse, http } from "msw";
import type { components } from "@/types/api.generated";

type SeriesListPage = components["schemas"]["SeriesListPage"];
type SeriesListItem = components["schemas"]["SeriesListItem"];
type SeriesDetail = components["schemas"]["SeriesDetail"];
type ReleasePage = components["schemas"]["ReleasePage"];
type ReleaseDto = components["schemas"]["ReleaseDto"];
type StatsResponse = components["schemas"]["StatsResponse"];

const NOW = Math.floor(Date.now() / 1000);

const SERIES: (SeriesListItem & {
  genres: string[];
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
    genres: ["action", "horror"],
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
    genres: ["isekai", "drama"],
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
    genres: ["action", "fantasy"],
    alternateTitles: [],
  },
];

export const handlers = [
  http.get("/api/v1/health", () => HttpResponse.json({ status: "ok" })),

  http.get("/api/v1/stats", () => {
    const stats: StatsResponse = {
      activeProvider: "mangabaka",
      series: SERIES.length,
      totalReleases: 12,
      releases: {
        resolved: 8,
        unresolved: 2,
        ambiguous: 1,
        reviewPending: 1,
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
        },
      ],
    }),
  ),

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
        },
      ],
    }),
  ),

  http.get("/api/v1/series", ({ request }) => {
    const url = new URL(request.url);
    const kind = url.searchParams.get("kind");
    const status = url.searchParams.get("status");
    const owned = url.searchParams.get("owned");
    const page = Number(url.searchParams.get("page") ?? "1");
    const pageSize = Number(url.searchParams.get("pageSize") ?? "24");

    let filtered = SERIES.slice();
    if (kind) filtered = filtered.filter((s) => s.kind === kind);
    if (status) filtered = filtered.filter((s) => s.status === status);
    if (owned === "true") filtered = filtered.filter((s) => s.owned === true);
    if (owned === "false") filtered = filtered.filter((s) => s.owned === false);

    const start = (page - 1) * pageSize;
    const items = filtered
      .slice(start, start + pageSize)
      .map(
        ({ genres: _g, alternateTitles: _a, ...rest }): SeriesListItem => rest,
      );
    const body: SeriesListPage = {
      items,
      page,
      pageSize,
      total: filtered.length,
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
      owned: found.owned,
      genres: found.genres,
      externalIds: [
        {
          provider: "mangabaka",
          externalId: String(found.id * 1111),
          externalUrl: null,
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
];
