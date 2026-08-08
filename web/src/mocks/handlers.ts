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
type ReleaseGroupsResponse = components["schemas"]["ReleaseGroupsResponse"];
type LinkRequest = components["schemas"]["LinkRequest"];
type CreateSeriesRequest = components["schemas"]["CreateSeriesRequest"];
type UpdateSeriesRequest = components["schemas"]["UpdateSeriesRequest"];
type BulkReviewRequest = components["schemas"]["BulkReviewRequest"];
type BulkLinkRequest = components["schemas"]["BulkLinkRequest"];
type TagList = components["schemas"]["TagList"];

const NOW = Math.floor(Date.now() / 1000);
const ADMIN_TOKEN = "test-admin-token";

type MockSeries = SeriesListItem & {
  genres: string[];
  tags: string[];
  alternateTitles: string[];
};
// Literals omit `wishlisted` for brevity; the `.map` below defaults every
// fixture row to not-wishlisted. The PUT /wishlist handler flips it in place.
const SERIES: MockSeries[] = (
  [
    {
      id: 1,
      canonicalTitle: "Chainsaw Man",
      coverUrl: "mock",
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
      coverUrl: "mock",
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
      coverUrl: "mock",
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
    // --- Codex ownership states, for previewing the badge/border treatment ---
    // `owned` is derived from the presence of `codex` on the real backend, so
    // every entry below keeps the two in sync.
    // complete: owned on Codex and current. The state that should recede.
    {
      id: 4,
      canonicalTitle: "Zero Damage Sword Saint",
      coverUrl: "mock",
      firstSeenAt: NOW - 86_400 * 10,
      lastReleaseAt: NOW - 86_400 * 2,
      kind: "manga",
      status: "ongoing",
      year: 2024,
      owned: true,
      description:
        "A washed-up swordsman reincarnates as the weakest stat line imaginable and turns zero attack power into an unbeatable defense.",
      genres: ["action", "fantasy"],
      tags: ["reincarnation", "op protagonist"],
      alternateTitles: ["攻撃力ゼロから始める剣聖譚"],
      metadataSource: "offline_cache",
      releaseCount: 1,
      highestVolume: 3,
      totalVolumes: 3,
      codex: {
        status: "complete",
        deepLink: "https://codex.example/series/zero-damage-sword-saint",
        linkKind: "auto",
        seriesUuid: "00000000-0000-0000-0000-000000000004",
        syncedAt: NOW - 1_800,
        localMaxVolume: 3,
        volumesOwned: 3,
      },
    },
    // behind: owned, but newer volumes/chapters have surfaced. The only
    // genuinely actionable state; this is the one that should pop.
    {
      id: 5,
      canonicalTitle: "Jujutsu Kaisen",
      coverUrl: "mock",
      firstSeenAt: NOW - 86_400 * 200,
      lastReleaseAt: NOW - 86_400 * 2,
      kind: "manga",
      status: "completed",
      year: 2018,
      owned: true,
      description:
        "A high-schooler swallows a cursed finger to save his friends and is dragged into a world of jujutsu sorcerers and curses.",
      genres: ["action", "supernatural"],
      tags: ["curses", "shonen"],
      alternateTitles: ["呪術廻戦"],
      metadataSource: "offline_cache",
      releaseCount: 10,
      highestVolume: 30,
      totalVolumes: 30,
      highestChapter: 271,
      totalChapters: 272,
      codex: {
        status: "behind",
        deepLink: "https://codex.example/series/jujutsu-kaisen",
        linkKind: "auto",
        seriesUuid: "00000000-0000-0000-0000-000000000005",
        syncedAt: NOW - 1_800,
        localMaxVolume: 24,
        localMaxChapter: 210,
        volumesOwned: 24,
      },
    },
    // present: owned on Codex, but we can't tell whether it's current (no
    // numbered releases to compare against). Stays quiet.
    {
      id: 6,
      canonicalTitle: "Owl Night",
      coverUrl: "mock",
      firstSeenAt: NOW - 86_400 * 5,
      lastReleaseAt: NOW - 3_600 * 2,
      kind: "manga",
      status: "ongoing",
      year: 2021,
      owned: true,
      description:
        "A nocturnal courier ferries secrets across a city that never sleeps.",
      genres: ["mystery", "drama"],
      tags: ["urban"],
      alternateTitles: ["アウルナイト"],
      metadataSource: "offline_cache",
      releaseCount: 3,
      codex: {
        status: "present",
        deepLink: "https://codex.example/series/owl-night",
        linkKind: "manual",
        seriesUuid: "00000000-0000-0000-0000-000000000006",
        syncedAt: NOW - 1_800,
        volumesOwned: 4,
      },
    },
    // Un-owned context: the default state of a discovery feed. No codex
    // overlay, no badge, no border accent.
    {
      id: 7,
      canonicalTitle: "My Tiny Senpai",
      coverUrl: "mock",
      firstSeenAt: NOW - 86_400 * 6,
      lastReleaseAt: NOW - 86_400 * 2,
      kind: "manga",
      status: "ongoing",
      year: 2020,
      owned: false,
      description:
        "A doting senpai and her hopeless kouhai, one office at a time.",
      genres: ["romance", "comedy"],
      tags: ["office", "slice of life"],
      alternateTitles: ["うちの会社の小さい先輩の話"],
      metadataSource: "offline_cache",
      releaseCount: 1,
      highestVolume: 5,
      totalVolumes: 9,
    },
    {
      id: 8,
      canonicalTitle: "Destroy All Humans. They Can't Be Regenerated.",
      coverUrl: "mock",
      firstSeenAt: NOW - 86_400 * 8,
      lastReleaseAt: NOW - 86_400 * 2,
      kind: "manga",
      status: "completed",
      year: 2018,
      owned: false,
      description: "A deadpan office comedy with an unreasonably long title.",
      genres: ["comedy"],
      tags: ["office"],
      alternateTitles: [],
      metadataSource: "offline_cache",
      releaseCount: 2,
      highestVolume: 7,
      totalVolumes: 18,
    },
    {
      id: 9,
      canonicalTitle: "Ichi the Witch",
      coverUrl: "mock",
      firstSeenAt: NOW - 86_400 * 4,
      lastReleaseAt: NOW - 86_400 * 2,
      kind: "manga",
      status: "ongoing",
      year: 2024,
      owned: false,
      description:
        "A boy raised by witches hunts down the magic that wronged him.",
      genres: ["action", "fantasy"],
      tags: ["witches", "shonen"],
      alternateTitles: ["魔男のイチ"],
      metadataSource: "offline_cache",
      releaseCount: 2,
      highestVolume: 2,
      totalVolumes: 8,
      highestChapter: 68,
      totalChapters: 83,
    },
    {
      // A manual (operator-authored) series, used by the edit + manual/auto
      // filter tests. The PATCH handler mutates this row in place, so the
      // fixture is snapshot/restored by `resetSeries()`.
      id: 10,
      canonicalTitle: "Obscure Doujin Anthology",
      coverUrl: null,
      firstSeenAt: NOW - 86_400 * 3,
      lastReleaseAt: NOW - 86_400,
      kind: "manga",
      status: null,
      year: 2023,
      owned: false,
      description: "A manual catalog entry MangaBaka lacks.",
      genres: [],
      tags: [],
      alternateTitles: [],
      metadataSource: "manual",
      releaseCount: 1,
    },
  ] as Omit<MockSeries, "wishlisted">[]
).map((s) => ({ ...s, wishlisted: false }));

// Deep snapshot of the seed catalog. The series handlers (create, edit)
// mutate `SERIES` in place, so tests restore it via `resetSeries()`.
const INITIAL_SERIES = SERIES.map((s) => ({
  ...s,
  genres: [...s.genres],
  tags: [...s.tags],
  alternateTitles: [...s.alternateTitles],
}));

// Maps a `${provider}:${externalId}` add-from-provider request to the series id
// it created, so a re-add is idempotent (mirrors the backend's
// series_external_ids lookup). Reset alongside SERIES.
const fromProviderIndex = new Map<string, number>();

/// Clip fixture rows directly (bypassing the PUT handler) so wishlist-page
/// tests can start from a populated list.
export function seedWishlisted(ids: number[]) {
  for (const id of ids) {
    const found = SERIES.find((s) => s.id === id);
    if (found) {
      found.wishlisted = true;
      found.wishlistedAt = NOW;
    }
  }
}

export function resetSeries() {
  SERIES.length = 0;
  fromProviderIndex.clear();
  for (const s of INITIAL_SERIES) {
    SERIES.push({
      ...s,
      genres: [...s.genres],
      tags: [...s.tags],
      alternateTitles: [...s.alternateTitles],
    });
  }
}

// Maps a discovery-source feed name to the mock series it has linked a release
// to. The real backend derives this from the releases table (distinct
// `source_name` over linked rows); the mock keeps a static map so the admin
// source filter and its `with-series-count` enumeration have data to exercise.
const SOURCE_SERIES: Record<string, number[]> = {
  "english-manga-trusted": [1, 5, 9],
  tsuna69: [3, 7],
  "popular-uploads": [6, 8],
};

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
    informationUrl: "https://sevenseasentertainment.com/series/mystery-series/",
    commentSuggestedLinks: {
      mangaupdates:
        "https://www.mangaupdates.com/series/ylx5wzn/mystery-series",
    },
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
  // The series catalog is mutated by the create/edit handlers; restore it
  // alongside the queue so every test starts from the same fixtures.
  resetSeries();
}

/// Replace the unresolved queue with caller-supplied rows. Tests that need a
/// specific size or ordering (e.g. exercising shift+click range selection,
/// which is meaningless with the two-row default) seed it directly.
export function seedReviewQueue(items: UnresolvedRelease[]) {
  queue = items.map((r) => ({
    ...r,
    candidates: r.candidates.map((c) => ({ ...c })),
  }));
}

/// Build an unresolved release off the simple (no-candidate) template,
/// overriding only what the test cares about. `id` is required so each row is
/// individually addressable via its `select-${id}` testid.
export function makeUnresolved(
  id: string,
  overrides: Partial<UnresolvedRelease> = {},
): UnresolvedRelease {
  return { ...INITIAL_QUEUE[1], candidates: [], ...overrides, id };
}

// The slice of `searchQueries` a given breadth considers: breadth 1 = the
// primary `[0]`, 2 = `[0..2)`, 3 (or anything else) = all. Mirrors the
// server's `breadth_key_bound`.
function breadthVariants(searchQueries: string[], breadth: number): string[] {
  if (breadth >= 3) return searchQueries;
  return searchQueries.slice(0, Math.max(1, breadth));
}

// Whether a release belongs to the group keyed by `searchQuery` at `breadth`.
function inGroup(r: UnresolvedRelease, searchQuery: string, breadth: number) {
  return breadthVariants(r.searchQueries, breadth).includes(searchQuery);
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
  const searchQuery = body.searchQuery?.trim();
  const breadth = body.breadth ?? 1;
  return queue.filter((r) => {
    if (q && !r.title.toLowerCase().includes(q)) return false;
    if (body.sourceName && r.sourceName !== body.sourceName) return false;
    if (body.format && !r.formats.includes(body.format)) return false;
    if (body.status && QUEUE_STATUSES.includes(body.status)) {
      if (r.resolutionStatus !== body.status) return false;
    }
    if (searchQuery && !inGroup(r, searchQuery, breadth)) return false;
    return true;
  });
}

// --- Per-series release search state -----------------------------------

interface SearchEntryMock {
  name: string;
  kind: string;
  default: boolean;
  searchUrl: string;
  maxPages: number;
  fetchDetails: boolean;
}

interface SearchRunMock {
  id: number;
  ranAt: number;
  finishedAt: number | null;
  searchName: string;
  seriesId: number;
  trigger: string;
  outcome: string;
  queriesAttempted: number | null;
  pagesFetched: number | null;
  releasesSeen: number | null;
  releasesNew: number | null;
  error: string | null;
  /// Used by the global timeline endpoint; per-series responses omit it.
  seriesTitle?: string;
}

interface SourceRunMock {
  id: number;
  startedAt: number;
  finishedAt: number | null;
  status: string;
  trigger: string;
  fetchedCount: number | null;
  newCount: number | null;
  resolvedCount: number | null;
  errorKind: string | null;
  errorMessage: string | null;
  fetchDurationMs: number | null;
  enrichDurationMs: number | null;
  resolveDurationMs: number | null;
  progressPhase: string | null;
}

/// Default per-source timeline shown in dev mock mode for any source
/// that hasn't been explicitly seeded: one clean cron run and one failure.
const DEFAULT_SOURCE_RUNS: SourceRunMock[] = [
  {
    id: 2,
    startedAt: NOW - 1800,
    finishedAt: NOW - 1790,
    status: "failure",
    trigger: "manual",
    fetchedCount: null,
    newCount: null,
    resolvedCount: null,
    errorKind: "timeout",
    errorMessage: "nyaa.si timed out after 30s",
    fetchDurationMs: null,
    enrichDurationMs: null,
    resolveDurationMs: null,
    progressPhase: null,
  },
  {
    id: 1,
    startedAt: NOW - 7200,
    finishedAt: NOW - 7140,
    status: "success",
    trigger: "cron",
    fetchedCount: 75,
    newCount: 4,
    resolvedCount: 3,
    errorKind: null,
    errorMessage: null,
    fetchDurationMs: 1200,
    enrichDurationMs: 8400,
    resolveDurationMs: 2100,
    progressPhase: null,
  },
];

let sourceRunsByName: Record<string, SourceRunMock[]> = {};

export function seedSourceRuns(name: string, runs: SourceRunMock[]) {
  sourceRunsByName[name] = runs;
}

export function resetSourceRuns() {
  sourceRunsByName = {};
}

const DEFAULT_SEARCH_ENTRIES: SearchEntryMock[] = [
  {
    name: "Nyaa Literature - Eng",
    kind: "nyaa",
    default: true,
    searchUrl: "https://nyaa.si/?f=0&c=3_1",
    maxPages: 5,
    fetchDetails: true,
  },
  {
    name: "Nyaa Literature - Raw",
    kind: "nyaa",
    default: false,
    searchUrl: "https://nyaa.si/?f=0&c=3_3",
    maxPages: 5,
    fetchDetails: true,
  },
];

let searchEntries: SearchEntryMock[] = [...DEFAULT_SEARCH_ENTRIES];
let searchRuns: SearchRunMock[] = [];
let nextSearchRunId = 1;
let searchBusy = false;
/// `releasesNew` stamped on a completed mock run; tests assert on it.
const searchResultNewCount = 3;

export function seedSearchEntries(entries: SearchEntryMock[]) {
  searchEntries = entries;
}

export function seedSearchRuns(runs: SearchRunMock[]) {
  searchRuns = runs;
  nextSearchRunId = Math.max(0, ...runs.map((r) => r.id)) + 1;
}

/// Makes the trigger endpoint answer `skipped` (a walk already in flight).
export function setSearchBusy(busy: boolean) {
  searchBusy = busy;
}

export function resetSearch() {
  searchEntries = [...DEFAULT_SEARCH_ENTRIES];
  searchRuns = [];
  nextSearchRunId = 1;
  searchBusy = false;
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

  // Cover proxy: the real backend fetches + caches the upstream cover. In
  // mock mode we redirect to Lorem Picsum (deterministic per series id) so
  // the feed shows real, varied art — useful for judging overlays/badges
  // against busy images rather than a flat placeholder.
  http.get("/api/v1/covers/:id", ({ params }) =>
    HttpResponse.redirect(
      `https://picsum.photos/seed/tsundoku-${params.id}/360/480`,
      302,
    ),
  ),

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
          foreignSources: ["mangaupdates", "mal", "anilist", "mangadex"],
          lastRefresh: {
            fetchedAt: NOW - 3_600,
            cacheVersion: "abc123",
            recordCount: 585_000,
            sourceUrl:
              "https://api.mangabaka.org/v1/database/series.sqlite.tar.gz",
            bytesDownloaded: 476_000_000,
          },
          config: {
            apiFallback: true,
            apiKeySet: true,
            apiBaseUrl: "https://api.mangabaka.org",
            offlineDumpUrl:
              "https://api.mangabaka.org/v1/database/series.sqlite.tar.gz",
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
          description: "A canned synopsis for the first hit.",
          genres: ["Action", "Horror"],
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
          description: null,
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
        // Second source carries `inFlight` so component tests can exercise
        // the DTO-driven pill path (page mount with no SSE replay).
        {
          kind: "nyaa",
          name: "running-on-load",
          lastPolledAt: NOW - 86_400,
          lastSuccessAt: NOW - 86_400,
          lastError: null,
          lastSummary: "10 new releases",
          config: {
            enabled: true,
            cron: "0 */6 * * *",
            feedUrl: "https://nyaa.si/?page=rss&u=someone",
            fetchDetails: true,
            timeoutSeconds: 30,
            siteBaseUrl: "https://nyaa.si",
            maxPages: 1,
          },
          inFlight: { startedAt: NOW - 30 },
        },
        // Third source carries `inFlight.progress` so component tests can
        // exercise the numeric-progress rendering path.
        {
          kind: "nyaa",
          name: "running-with-progress",
          lastPolledAt: NOW - 86_400 * 2,
          lastSuccessAt: NOW - 86_400 * 2,
          lastError: null,
          lastSummary: "many releases",
          config: {
            enabled: true,
            cron: "0 */6 * * *",
            feedUrl: "https://nyaa.si/?page=rss&u=anotherone",
            fetchDetails: true,
            timeoutSeconds: 30,
            siteBaseUrl: "https://nyaa.si",
            maxPages: 1,
          },
          inFlight: {
            startedAt: NOW - 10,
            progress: { current: 47, total: 200 },
          },
        },
      ],
    }),
  ),

  // Source-filter dropdown vocab. Registered before any `/sources/:param`
  // sibling so MSW's first-match-wins can't shadow it (static-before-param
  // convention). Admin-only on the real backend; the mock returns it
  // unconditionally so component tests can render the control.
  http.get("/api/v1/sources/with-series-count", () => {
    const body: TagList = {
      items: Object.entries(SOURCE_SERIES)
        .map(([name, ids]) => ({ name, seriesCount: new Set(ids).size }))
        .sort(
          (a, b) =>
            b.seriesCount - a.seriesCount || a.name.localeCompare(b.name),
        ),
    };
    return HttpResponse.json(body);
  }),

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

  http.post("/api/v1/releases/re-enrich", async ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const body = (await request.json()) as {
      statuses?: string[];
      onlyMissingDetails?: boolean;
      sources?: string[] | null;
    };
    return HttpResponse.json({
      statuses: body.statuses ?? [],
      onlyMissingDetails: body.onlyMissingDetails ?? false,
      sources: body.sources ?? null,
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
      wishlisted: false,
    });
    const detail: SeriesDetail = {
      wishlisted: false,
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

  // Add a series straight from a provider (the wishlist "Add from MangaBaka"
  // flow). Idempotent on `${provider}:${externalId}` → 200 with the existing
  // row, else 201 with a fresh one. Registered before the `:id` param siblings
  // so the static `/from-provider` path isn't shadowed.
  http.post("/api/v1/series/from-provider", async ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const body = (await request.json()) as {
      provider: string;
      externalId: string;
      wishlist?: boolean;
    };
    const provider = (body.provider ?? "").trim();
    const externalId = (body.externalId ?? "").trim();
    if (!provider || !externalId) {
      return new HttpResponse(
        JSON.stringify({
          error: "bad_request",
          message: "provider and externalId must not be empty",
        }),
        { status: 400, headers: { "content-type": "application/json" } },
      );
    }
    const key = `${provider}:${externalId}`;
    const existingId = fromProviderIndex.get(key);
    const wishlist = body.wishlist ?? true;
    const makeDetail = (row: MockSeries): SeriesDetail => ({
      wishlisted: row.wishlisted,
      wishlistedAt: row.wishlistedAt,
      id: row.id,
      canonicalTitle: row.canonicalTitle,
      alternateTitles: row.alternateTitles,
      coverUrl: row.coverUrl,
      kind: row.kind,
      status: row.status,
      year: row.year,
      description: row.description,
      owned: row.owned,
      genres: row.genres,
      tags: row.tags,
      externalIds: [{ provider, externalId, fetchedAt: NOW }],
      firstSeenAt: row.firstSeenAt,
      lastReleaseAt: row.lastReleaseAt,
      metadataFetchedAt: NOW,
      metadataSource: row.metadataSource,
      highestVolume: null,
      highestChapter: null,
    });
    if (existingId != null) {
      const row = SERIES.find((s) => s.id === existingId);
      if (row) {
        if (wishlist) {
          row.wishlisted = true;
          row.wishlistedAt = NOW;
        }
        return HttpResponse.json(makeDetail(row), { status: 200 });
      }
    }
    const id = Math.max(0, ...SERIES.map((s) => s.id)) + 1;
    const row: MockSeries = {
      id,
      canonicalTitle: `MangaBaka series ${externalId}`,
      coverUrl: null,
      firstSeenAt: NOW,
      lastReleaseAt: NOW,
      kind: "manga",
      status: "ongoing",
      year: null,
      owned: false,
      description: null,
      genres: [],
      tags: [],
      alternateTitles: [],
      metadataSource: "api",
      releaseCount: 0,
      wishlisted: wishlist,
      wishlistedAt: wishlist ? NOW : null,
    };
    SERIES.push(row);
    fromProviderIndex.set(key, id);
    return HttpResponse.json(makeDetail(row), { status: 201 });
  }),

  // Edit a manual series. Mirrors the backend: 409 for provider-backed rows,
  // 404 for unknown ids, 400 for an empty title. PATCH on `:id` is a distinct
  // method from the POST static siblings, so ordering is not a concern here.
  http.patch("/api/v1/series/:id", async ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const id = Number(params.id);
    const found = SERIES.find((s) => s.id === id);
    if (!found) return new HttpResponse(null, { status: 404 });
    if (found.metadataSource !== "manual") {
      return new HttpResponse(
        JSON.stringify({
          error: "conflict",
          message: `series ${id} is provider-backed; only manual series can be edited`,
        }),
        { status: 409, headers: { "content-type": "application/json" } },
      );
    }
    const body = (await request.json()) as UpdateSeriesRequest;
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
    const alternateTitles = (body.alternateTitles ?? [])
      .map((t) => t.trim())
      .filter((t) => t.length > 0);
    found.canonicalTitle = title;
    found.alternateTitles = alternateTitles;
    found.kind = body.kind?.trim() || null;
    found.status = body.status?.trim() || null;
    found.year = typeof body.year === "number" ? body.year : null;
    found.coverUrl = body.coverUrl?.trim() || null;
    found.description = body.description?.trim() || null;
    const detail: SeriesDetail = {
      wishlisted: false,
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
      externalIds: [],
      firstSeenAt: found.firstSeenAt,
      lastReleaseAt: found.lastReleaseAt,
      metadataFetchedAt: NOW,
      metadataSource: "manual",
      highestVolume: null,
      highestChapter: null,
    };
    return HttpResponse.json(detail);
  }),

  // Static path registered before any `:id`-style sibling so MSW's matcher
  // can't shadow it (see memory: MSW static vs param route ordering).
  http.post("/api/v1/series/invalidate-metadata-hashes", ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const url = new URL(request.url);
    const provider = url.searchParams.get("provider");
    return HttpResponse.json({
      provider,
      invalidated: 12,
      skippedManual: 1,
    });
  }),

  // Stub the cover-proxy invalidate endpoint so the Maintenance card has
  // something to talk to in mock + test runs. The proxy GETs themselves
  // (`/api/v1/covers/{id}`, `/api/v1/covers/by-url`) are intentionally
  // not mocked: in dev the Vite proxy reaches the real backend, and in
  // tests the component tree we exercise never renders a cover.
  http.post("/api/v1/covers/invalidate-cache", ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    return HttpResponse.json({
      filesDeleted: 7,
      bytesFreed: 245_678,
    });
  }),

  http.post("/api/v1/series/refresh-all", ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const all = new URL(request.url).searchParams.get("all") === "true";
    return HttpResponse.json({
      provider: "mangabaka",
      triggered: true,
      skipped: false,
      batchSize: 25,
      minAgeDays: all ? 0 : 7,
      scope: all ? "all" : "settings",
    });
  }),

  http.post("/api/v1/series/recompute-spans", ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    return HttpResponse.json({
      releasesRewritten: 18,
      seriesUpdated: 5,
    });
  }),

  // --- Series bulk actions -------------------------------------------------
  // All three are static `bulk` segments registered before their
  // `/series/:id/...` param siblings (MSW is first-match-wins, unlike axum;
  // see memory: MSW static vs param route ordering).

  http.put("/api/v1/series/bulk/wishlist", async ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const body = (await request.json()) as {
      ids: number[];
      wishlisted: boolean;
    };
    if (!body.ids || body.ids.length === 0) {
      return new HttpResponse(
        JSON.stringify({
          error: "bad_request",
          message: "ids must not be empty",
        }),
        { status: 400, headers: { "content-type": "application/json" } },
      );
    }
    let updated = 0;
    for (const id of body.ids) {
      const found = SERIES.find((s) => s.id === id);
      if (!found) continue;
      found.wishlisted = body.wishlisted;
      found.wishlistedAt = body.wishlisted ? NOW : null;
      updated += 1;
    }
    return HttpResponse.json({ updated });
  }),

  http.post("/api/v1/series/bulk/refresh-metadata", async ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const body = (await request.json()) as { ids: number[] };
    if (!body.ids || body.ids.length === 0) {
      return new HttpResponse(
        JSON.stringify({
          error: "bad_request",
          message: "ids must not be empty",
        }),
        { status: 400, headers: { "content-type": "application/json" } },
      );
    }
    let refreshed = 0;
    const skipped: { id: number; reason: string }[] = [];
    for (const id of body.ids) {
      const found = SERIES.find((s) => s.id === id);
      if (!found) {
        skipped.push({ id, reason: "series not found" });
      } else if (found.metadataSource === "manual") {
        // Mirrors the backend: manual rows carry no active-provider mapping.
        skipped.push({
          id,
          reason: 'no mapping for active provider "mangabaka"',
        });
      } else {
        refreshed += 1;
      }
    }
    return HttpResponse.json({ refreshed, skipped });
  }),

  http.post("/api/v1/series/bulk/search-releases", async ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const body = (await request.json()) as { ids: number[]; search?: string };
    if (!body.ids || body.ids.length === 0) {
      return new HttpResponse(
        JSON.stringify({
          error: "bad_request",
          message: "ids must not be empty",
        }),
        { status: 400, headers: { "content-type": "application/json" } },
      );
    }
    if (searchEntries.length === 0) {
      return new HttpResponse(
        JSON.stringify({
          error: "misconfigured",
          message: "no [[search]] entries configured",
        }),
        { status: 503, headers: { "content-type": "application/json" } },
      );
    }
    const entry = body.search
      ? searchEntries.find((e) => e.name === body.search)
      : (searchEntries.find((e) => e.default) ?? searchEntries[0]);
    if (!entry) {
      return new HttpResponse(
        JSON.stringify({
          error: "bad_request",
          message: `unknown search entry ${JSON.stringify(body.search)}`,
        }),
        { status: 400, headers: { "content-type": "application/json" } },
      );
    }
    const existing = body.ids.filter((id) => SERIES.some((s) => s.id === id));
    if (existing.length === 0) {
      return new HttpResponse(
        JSON.stringify({
          error: "not_found",
          message: "none of the listed series exist",
        }),
        { status: 404, headers: { "content-type": "application/json" } },
      );
    }
    if (searchBusy) {
      return HttpResponse.json({
        search: entry.name,
        matched: existing.length,
        triggered: false,
        skipped: true,
      });
    }
    // One completed-shortly run per series, like the single trigger mock.
    for (const seriesId of existing) {
      const run: SearchRunMock = {
        id: nextSearchRunId++,
        ranAt: NOW,
        finishedAt: null,
        searchName: entry.name,
        seriesId,
        trigger: "manual",
        outcome: "running",
        queriesAttempted: null,
        pagesFetched: null,
        releasesSeen: null,
        releasesNew: null,
        error: null,
      };
      searchRuns.unshift(run);
      setTimeout(() => {
        run.outcome = "success";
        run.finishedAt = NOW + 2;
        run.queriesAttempted = 3;
        run.pagesFetched = 4;
        run.releasesSeen = 41;
        run.releasesNew = searchResultNewCount;
      }, 700);
    }
    return HttpResponse.json({
      search: entry.name,
      matched: existing.length,
      triggered: true,
      skipped: false,
    });
  }),

  http.post("/api/v1/series/:id/refresh-metadata", ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const id = Number(params.id);
    const found = SERIES.find((s) => s.id === id);
    if (!found) return new HttpResponse(null, { status: 404 });
    const body: SeriesDetail = {
      wishlisted: found.wishlisted,
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
      // Carry the Codex overlay onto the detail payload so the admin detail
      // page renders the ownership badge (it gates on `s.codex`).
      codex: found.codex ?? null,
    };
    return HttpResponse.json(body);
  }),

  http.put(
    "/api/v1/series/:id/ignore-completion",
    async ({ request, params }) => {
      const denied = requireAdmin(request);
      if (denied) return denied;
      const id = Number(params.id);
      const found = SERIES.find((s) => s.id === id);
      if (!found) return new HttpResponse(null, { status: 404 });
      const body = (await request.json()) as { ignore: boolean };
      // Mirror the backend's short-circuit: when ignored, the status becomes
      // `ignored`; clearing it falls back to `behind` for this fixture.
      // Replace the codex reference (don't mutate it in place) — `INITIAL_SERIES`
      // shares the nested codex object, so an in-place edit would survive
      // `resetSeries()` and leak across tests.
      if (found.codex) {
        found.codex = {
          ...found.codex,
          status: body.ignore ? "ignored" : "behind",
        };
      }
      const detail: SeriesDetail = {
        wishlisted: found.wishlisted,
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
        externalIds: [],
        firstSeenAt: found.firstSeenAt,
        lastReleaseAt: found.lastReleaseAt,
        metadataFetchedAt: NOW,
        metadataSource: found.metadataSource,
        highestVolume: null,
        highestChapter: null,
        codex: found.codex ?? null,
      };
      return HttpResponse.json(detail);
    },
  ),

  http.put("/api/v1/series/:id/wishlist", async ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const id = Number(params.id);
    const found = SERIES.find((s) => s.id === id);
    if (!found) return new HttpResponse(null, { status: 404 });
    const body = (await request.json()) as { wishlisted: boolean };
    found.wishlisted = body.wishlisted;
    found.wishlistedAt = body.wishlisted ? NOW : null;
    const detail: SeriesDetail = {
      wishlisted: found.wishlisted,
      wishlistedAt: found.wishlistedAt,
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
      externalIds: [],
      firstSeenAt: found.firstSeenAt,
      lastReleaseAt: found.lastReleaseAt,
      metadataFetchedAt: NOW,
      metadataSource: found.metadataSource,
      highestVolume: null,
      highestChapter: null,
      codex: found.codex ?? null,
    };
    return HttpResponse.json(detail);
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
    const metadataSource = url.searchParams.get("metadataSource");
    const q = url.searchParams.get("q");
    const page = Number(url.searchParams.get("page") ?? "1");
    const pageSize = Number(url.searchParams.get("pageSize") ?? "24");

    // Kind / status are comma-separated and OR-combined, mirroring the
    // backend's `IN` filter (a single value is just a one-element set).
    const splitOr = (s: string | null) =>
      s
        ?.split(",")
        .map((p) => p.trim())
        .filter((p) => p.length > 0) ?? [];
    const kinds = splitOr(kind);
    const statuses = splitOr(status);

    let filtered = SERIES.slice();
    if (kinds.length > 0)
      filtered = filtered.filter(
        (s) => s.kind != null && kinds.includes(s.kind),
      );
    if (statuses.length > 0)
      filtered = filtered.filter(
        (s) => s.status != null && statuses.includes(s.status),
      );
    if (metadataSource === "manual")
      filtered = filtered.filter((s) => s.metadataSource === "manual");
    else if (metadataSource === "auto")
      filtered = filtered.filter((s) => s.metadataSource !== "manual");
    if (owned === "true") filtered = filtered.filter((s) => s.owned === true);
    if (owned === "false") filtered = filtered.filter((s) => s.owned === false);
    if (hasReleases === "true")
      filtered = filtered.filter((s) => s.releaseCount > 0);
    if (hasReleases === "false")
      filtered = filtered.filter((s) => s.releaseCount === 0);
    // Source filter (admin-only on the real backend): keep series with a
    // linked release from any selected feed. OR-combined, like kind/status.
    const sources = splitOr(url.searchParams.get("source"));
    if (sources.length > 0) {
      const allowed = new Set(
        sources.flatMap((name) => SOURCE_SERIES[name] ?? []),
      );
      filtered = filtered.filter((s) => allowed.has(s.id));
    }
    const wishlisted = url.searchParams.get("wishlisted");
    if (wishlisted === "true")
      filtered = filtered.filter((s) => s.wishlisted === true);
    if (wishlisted === "false")
      filtered = filtered.filter((s) => s.wishlisted === false);
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
      // A sweep has run, so the feed renders the Codex ownership badges /
      // border accents for items that carry a `codex` overlay.
      codexSyncedAt: NOW - 1_800,
    };
    return HttpResponse.json(body);
  }),

  // Catalog export: returns a tiny file payload with the same content-type +
  // Content-Disposition the backend sets, so the download helper exercises its
  // full path (blob + filename). Tests that assert on the request URL override
  // this with `server.use`.
  http.get("/api/v1/series/export", ({ request }) => {
    const url = new URL(request.url);
    const format = url.searchParams.get("format") ?? "json";
    const ext = format === "markdown" ? "md" : format;
    const body =
      format === "csv"
        ? "canonicalTitle\r\n"
        : format === "markdown"
          ? "# tsundoku series catalog\n"
          : "[]";
    const type =
      format === "csv"
        ? "text/csv"
        : format === "markdown"
          ? "text/markdown"
          : "application/json";
    return new HttpResponse(body, {
      headers: {
        "content-type": `${type}; charset=utf-8`,
        "content-disposition": `attachment; filename="tsundoku-series-export-2026-06-08.${ext}"`,
      },
    });
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

  // Static route: MUST stay registered before the `/series/:id` sibling
  // below, or the param route captures "lookup" (MSW is first-match-wins,
  // unlike axum's static-segment priority). Mirrors the backend's
  // series_external_ids resolution against the detail handler's synthetic
  // mangabaka ids (`id * 1111`).
  http.get("/api/v1/series/lookup", ({ request }) => {
    const url = new URL(request.url);
    const provider = (url.searchParams.get("provider") ?? "")
      .trim()
      .toLowerCase();
    const externalId = (url.searchParams.get("externalId") ?? "").trim();
    if (!provider || !externalId)
      return new HttpResponse(null, { status: 400 });
    const numeric = Number(externalId);
    const seriesId = numeric / 1111;
    const found =
      provider === "mangabaka" && Number.isInteger(seriesId)
        ? SERIES.find((s) => s.id === seriesId)
        : undefined;
    if (!found)
      return HttpResponse.json(
        {
          error: "not_found",
          message: `series for ${provider}:${externalId}`,
        },
        { status: 404 },
      );
    return HttpResponse.json({ seriesId: found.id });
  }),

  http.get("/api/v1/series/:id", ({ params }) => {
    const id = Number(params.id);
    const found = SERIES.find((s) => s.id === id);
    if (!found) return new HttpResponse(null, { status: 404 });
    const body: SeriesDetail = {
      wishlisted: found.wishlisted,
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
      // Carry the Codex overlay onto the detail payload so the admin detail
      // page renders the ownership badge (it gates on `s.codex`).
      codex: found.codex ?? null,
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
      {
        id: "nyaa:112",
        sourceKind: "nyaa",
        sourceName: "english-manga-trusted",
        externalId: "112",
        title: `${SERIES[0]?.canonicalTitle} v02 (Digital) (CBZ)`,
        link: "https://nyaa.si/view/112",
        magnet: "magnet:?xt=urn:btih:dummy2",
        torrentUrl: null,
        ddlUrl: null,
        infoHash: null,
        sizeBytes: 12_345_679,
        files: ["chainsaw_man_v02.cbz"],
        formats: ["cbz"],
        postedAt: NOW - 7_100,
        observedAt: NOW - 5_900,
        seriesId: 1,
        resolutionPath: "fuzzy_title",
        resolutionConfidence: 0.92,
        resolutionStatus: "resolved",
        resolutionAttempts: 1,
        lastResolveAttemptAt: NOW - 5_900,
      },
      {
        // Already sent: excluded from the bulk-select affordance.
        id: "nyaa:113",
        sourceKind: "nyaa",
        sourceName: "english-manga-trusted",
        externalId: "113",
        title: `${SERIES[0]?.canonicalTitle} v03 (Digital) (CBZ)`,
        link: "https://nyaa.si/view/113",
        magnet: "magnet:?xt=urn:btih:dummy3",
        torrentUrl: null,
        ddlUrl: null,
        infoHash: null,
        sizeBytes: 12_345_680,
        files: ["chainsaw_man_v03.cbz"],
        formats: ["cbz"],
        postedAt: NOW - 7_000,
        observedAt: NOW - 5_800,
        seriesId: 1,
        resolutionPath: "fuzzy_title",
        resolutionConfidence: 0.92,
        resolutionStatus: "resolved",
        resolutionAttempts: 1,
        lastResolveAttemptAt: NOW - 5_800,
        sentToClientAt: NOW - 3_000,
        sentToClientLabel: "manga",
      },
      {
        // A third sendable release after the already-sent one, so a shift-click
        // range (111 → 114) must skip 113.
        id: "nyaa:114",
        sourceKind: "nyaa",
        sourceName: "english-manga-trusted",
        externalId: "114",
        title: `${SERIES[0]?.canonicalTitle} v04 (Digital) (CBZ)`,
        link: "https://nyaa.si/view/114",
        magnet: "magnet:?xt=urn:btih:dummy4",
        torrentUrl: null,
        ddlUrl: null,
        infoHash: null,
        sizeBytes: 12_345_681,
        files: ["chainsaw_man_v04.cbz"],
        formats: ["cbz"],
        postedAt: NOW - 6_900,
        observedAt: NOW - 5_700,
        seriesId: 1,
        resolutionPath: "fuzzy_title",
        resolutionConfidence: 0.92,
        resolutionStatus: "resolved",
        resolutionAttempts: 1,
        lastResolveAttemptAt: NOW - 5_700,
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

  // Registered before `/releases/unresolved` so MSW's first-match-wins can't
  // let the broader route shadow this one (static-before-param convention).
  http.get("/api/v1/releases/unresolved/groups", ({ request }) => {
    const url = new URL(request.url);
    const q = url.searchParams.get("q")?.trim().toLowerCase();
    const sourceName = url.searchParams.get("sourceName");
    const format = url.searchParams.get("format");
    const status = url.searchParams.get("status");
    const breadth = Number(url.searchParams.get("breadth") ?? "1");
    const QUEUE_STATUSES = ["unresolved", "ambiguous", "review_pending"];
    // Same filters as the list, minus the group filter itself.
    const scoped = queue.filter((r) => {
      if (q && !r.title.toLowerCase().includes(q)) return false;
      if (sourceName && r.sourceName !== sourceName) return false;
      if (format && !r.formats.includes(format)) return false;
      if (status && QUEUE_STATUSES.includes(status)) {
        if (r.resolutionStatus !== status) return false;
      }
      return true;
    });
    // Count distinct releases per cleaned query (within the breadth bound) and
    // surface the most common candidate series as the hint.
    const counts = new Map<string, number>();
    const candidateCounts = new Map<string, Map<number, number>>();
    const candidateTitles = new Map<number, string>();
    for (const r of scoped) {
      const variants = new Set(breadthVariants(r.searchQueries, breadth));
      for (const v of variants) {
        counts.set(v, (counts.get(v) ?? 0) + 1);
        if (r.candidates.length > 0) {
          const perSeries = candidateCounts.get(v) ?? new Map<number, number>();
          for (const c of r.candidates) {
            perSeries.set(c.seriesId, (perSeries.get(c.seriesId) ?? 0) + 1);
            candidateTitles.set(c.seriesId, c.seriesTitle);
          }
          candidateCounts.set(v, perSeries);
        }
      }
    }
    const groups = [...counts.entries()]
      .filter(([, n]) => n > 1)
      .map(([query, count]) => {
        const perSeries = candidateCounts.get(query);
        let topCandidate: ReleaseGroupsResponse["groups"][number]["topCandidate"];
        if (perSeries && perSeries.size > 0) {
          const [seriesId] = [...perSeries.entries()].sort(
            (a, b) => b[1] - a[1] || a[0] - b[0],
          )[0];
          topCandidate = {
            seriesId,
            title: candidateTitles.get(seriesId) ?? "",
            coverUrl: null,
          };
        }
        return { query, count, topCandidate };
      })
      .sort((a, b) => b.count - a.count || a.query.localeCompare(b.query));
    const body: ReleaseGroupsResponse = { breadth, groups };
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
    const sort = url.searchParams.get("sort");
    const searchQuery = url.searchParams.get("searchQuery")?.trim();
    const breadth = Number(url.searchParams.get("breadth") ?? "1");
    const QUEUE_STATUSES = ["unresolved", "ambiguous", "review_pending"];
    const filtered = queue.filter((r) => {
      if (q && !r.title.toLowerCase().includes(q)) return false;
      if (sourceName && r.sourceName !== sourceName) return false;
      if (format && !r.formats.includes(format)) return false;
      // Mirror the server clamp: an out-of-queue status is ignored.
      if (status && QUEUE_STATUSES.includes(status)) {
        if (r.resolutionStatus !== status) return false;
      }
      // Release-group filter: AND with the title `q`, scoped by breadth.
      if (searchQuery && !inGroup(r, searchQuery, breadth)) return false;
      return true;
    });
    // Mirror the server ordering (case-insensitive title; recency otherwise).
    // The default / unknown case leaves the seed order intact.
    switch (sort) {
      case "title_asc":
        filtered.sort((a, b) =>
          a.title.toLowerCase().localeCompare(b.title.toLowerCase()),
        );
        break;
      case "title_desc":
        filtered.sort((a, b) =>
          b.title.toLowerCase().localeCompare(a.title.toLowerCase()),
        );
        break;
      case "observed_asc":
        filtered.sort((a, b) => a.observedAt - b.observedAt);
        break;
      case "observed_desc":
        filtered.sort((a, b) => b.observedAt - a.observedAt);
        break;
      case "posted_asc":
        filtered.sort((a, b) => a.postedAt - b.postedAt);
        break;
      case "posted_desc":
        filtered.sort((a, b) => b.postedAt - a.postedAt);
        break;
    }
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
  http.post("/api/v1/releases/import", async ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const { url } = (await request.json()) as { url: string };
    if (!url.startsWith("https://nyaa.si/")) {
      return new HttpResponse(
        JSON.stringify({
          error: "bad_request",
          message: `no configured search entry recognizes "${url}" as one of its post urls`,
        }),
        { status: 400, headers: { "content-type": "application/json" } },
      );
    }
    const externalId = url.split("/").pop() ?? "0";
    const alreadyKnown = externalId === "known";
    return HttpResponse.json({
      alreadyKnown,
      release: {
        ...INITIAL_KEPT[0],
        id: `nyaa:${externalId}`,
        externalId,
        title: "ReZero - Starting Life in Another World - Volume 01 [MTBBooks]",
        link: url,
        seriesId: 1,
        resolutionPath: "fuzzy_title",
        resolutionConfidence: 0.94,
        resolutionStatus: alreadyKnown ? "unresolved" : "resolved",
      },
    });
  }),

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

  // Link a selection of releases to one series; the server intersects with
  // the queue statuses, so the targets are exactly the matching `ids`. The
  // linked rows leave the queue.
  http.post("/api/v1/releases/bulk/link", async ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const body = (await request.json()) as BulkLinkRequest;
    const ids = new Set(body.ids ?? []);
    if (ids.size === 0) {
      return HttpResponse.json(
        { error: "bad_request", message: "`ids` must not be empty" },
        { status: 400 },
      );
    }
    const targets = queue.filter((r) => ids.has(r.id));
    queue = queue.filter((r) => !ids.has(r.id));
    // The mock doesn't materialize a provider series; echo back a stable id.
    const seriesId = body.seriesId ?? 1;
    return HttpResponse.json({ linked: targets.length, seriesId });
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

  // Codex presence integration (admin-only).
  http.get("/api/v1/codex/status", ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    return HttpResponse.json({
      enabled: true,
      reachable: true,
      codexName: "codex",
      codexVersion: "1.32.0",
      authState: "ok",
      lastPreflightAt: NOW,
      lastSuccessAt: NOW,
      linkedCount: 2,
      fetchedCount: 18,
      recentChecks: [
        { id: 1, checkedAt: NOW, reachable: true, trigger: "launch" },
      ],
      recentSyncRuns: [
        {
          id: 2,
          ranAt: NOW,
          trigger: "manual",
          outcome: "success",
          fetchedCount: 18,
          linkedCount: 2,
        },
        {
          id: 1,
          ranAt: NOW - 200,
          trigger: "cron",
          outcome: "auth_failed",
          error: "api_key rejected (401)",
        },
      ],
    });
  }),
  http.post("/api/v1/codex/refresh", ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    return HttpResponse.json({ triggered: true, skipped: false });
  }),
  http.post("/api/v1/codex/test", ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    // The mock client is "unreachable": a 200 report of reachable:false plus a
    // recorded manual history row.
    return HttpResponse.json({
      enabled: true,
      reachable: false,
      authState: "unknown",
      lastPreflightAt: NOW,
      lastError: "connection refused",
      recentChecks: [
        {
          id: 2,
          checkedAt: NOW,
          reachable: false,
          trigger: "manual",
          error: "connection refused",
        },
        { id: 1, checkedAt: NOW - 100, reachable: true, trigger: "launch" },
      ],
      recentSyncRuns: [],
    });
  }),

  // Send to torrent client (admin-only).
  http.get("/api/v1/download/status", ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    return HttpResponse.json({
      enabled: true,
      kind: "rutorrent",
      baseUrl: "https://box.example.com/rutorrent",
      hasCredentials: true,
      defaultLabel: "manga",
      defaultStart: true,
      preferTorrentFile: true,
      healthCron: "0 * * * *",
      reachable: true,
      lastTestAt: NOW,
      lastChangeAt: NOW,
      recentChecks: [
        { id: 1, checkedAt: NOW, reachable: true, trigger: "launch" },
      ],
      recentSends: [
        {
          id: 1,
          releaseId: "rel-1",
          releaseTitle: "Chainsaw Man v01",
          // No seriesId: the global mock feeds the router-less Download page
          // test, so the title renders as plain text (the linked variant is
          // exercised where a router context exists).
          sentAt: NOW,
          label: "manga",
          source: "torrent",
          success: true,
        },
      ],
    });
  }),
  http.post("/api/v1/download/test", ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    return HttpResponse.json({
      enabled: true,
      kind: "rutorrent",
      baseUrl: "https://box.example.com/rutorrent",
      hasCredentials: true,
      defaultLabel: "manga",
      defaultStart: true,
      preferTorrentFile: true,
      healthCron: "0 * * * *",
      reachable: false,
      lastTestAt: NOW,
      lastChangeAt: NOW,
      lastError: "connection refused",
      recentChecks: [
        {
          id: 1,
          checkedAt: NOW,
          reachable: false,
          trigger: "manual",
          error: "connection refused",
        },
      ],
      recentSends: [],
    });
  }),
  http.post("/api/v1/releases/:id/send-to-client", ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const id = String(params.id);
    // Reflect the send into whichever list holds the row (preserving each
    // list's element shape) so a refetch shows the "Sent" badge.
    const sent = { sentToClientAt: NOW, sentToClientLabel: "manga" };
    const qi = queue.findIndex((r) => r.id === id);
    if (qi >= 0) {
      queue[qi] = { ...queue[qi], ...sent };
      return HttpResponse.json(queue[qi]);
    }
    const ki = kept.findIndex((r) => r.id === id);
    if (ki >= 0) {
      kept[ki] = { ...kept[ki], ...sent };
      return HttpResponse.json(kept[ki]);
    }
    return new HttpResponse(
      JSON.stringify({ error: "not_found", message: `release ${id}` }),
      { status: 404, headers: { "content-type": "application/json" } },
    );
  }),

  // --- Per-series release search (admin-only) ------------------------------

  http.get("/api/v1/search/entries", ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    return HttpResponse.json({ items: searchEntries });
  }),

  http.post(
    "/api/v1/series/:id/search-releases",
    async ({ request, params }) => {
      const denied = requireAdmin(request);
      if (denied) return denied;
      const seriesId = Number(params.id);
      const body = (await request.json().catch(() => ({}))) as {
        search?: string;
      };
      if (searchEntries.length === 0) {
        return new HttpResponse(
          JSON.stringify({
            error: "misconfigured",
            message: "no [[search]] entries configured",
          }),
          { status: 503, headers: { "content-type": "application/json" } },
        );
      }
      const entry = body.search
        ? searchEntries.find((e) => e.name === body.search)
        : (searchEntries.find((e) => e.default) ?? searchEntries[0]);
      if (!entry) {
        return new HttpResponse(
          JSON.stringify({
            error: "bad_request",
            message: `unknown search entry ${JSON.stringify(body.search)}`,
          }),
          { status: 400, headers: { "content-type": "application/json" } },
        );
      }
      if (searchBusy) {
        return HttpResponse.json({
          search: entry.name,
          seriesId,
          triggered: false,
          skipped: true,
        });
      }
      const run: SearchRunMock = {
        id: nextSearchRunId++,
        ranAt: NOW,
        finishedAt: null,
        searchName: entry.name,
        seriesId,
        trigger: "manual",
        outcome: "running",
        queriesAttempted: null,
        pagesFetched: null,
        releasesSeen: null,
        releasesNew: null,
        error: null,
      };
      searchRuns.unshift(run);
      // Complete the walk shortly after, so the UI's poll cycle sees a
      // `running` row transition to `success` like the real backend.
      setTimeout(() => {
        run.outcome = "success";
        run.finishedAt = NOW + 2;
        run.queriesAttempted = 3;
        run.pagesFetched = 4;
        run.releasesSeen = 41;
        run.releasesNew = searchResultNewCount;
      }, 700);
      return HttpResponse.json({
        search: entry.name,
        seriesId,
        triggered: true,
        skipped: false,
      });
    },
  ),

  http.get("/api/v1/series/:id/search-runs", ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const seriesId = Number(params.id);
    return HttpResponse.json({
      items: searchRuns.filter((r) => r.seriesId === seriesId),
    });
  }),

  http.get("/api/v1/search/runs", ({ request }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    return HttpResponse.json({
      items: searchRuns.map((r) => ({
        ...r,
        seriesTitle: r.seriesTitle ?? `Series #${r.seriesId}`,
      })),
    });
  }),

  http.get("/api/v1/sources/:name/runs", ({ request, params }) => {
    const denied = requireAdmin(request);
    if (denied) return denied;
    const name = String(params.name);
    return HttpResponse.json({
      items: sourceRunsByName[name] ?? DEFAULT_SOURCE_RUNS,
    });
  }),
];
