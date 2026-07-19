import { MantineProvider } from "@mantine/core";
import { Notifications, notifications } from "@mantine/notifications";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { AdminShell } from "@/components/admin/AdminShell";
import {
  ADMIN_TEST_TOKEN,
  resetSourceRuns,
  seedSourceRuns,
} from "@/mocks/handlers";
import { ReviewPage } from "@/pages/ReviewPage";
import { useAdminAuth } from "@/stores/auth";
import { AdminCodexPage } from "./Codex";
import { AdminDownloadPage } from "./Download";
import { AdminIdMapsPage } from "./IdMaps";
import { AdminMaintenancePage } from "./Maintenance";
import { AdminMetricsPage } from "./Metrics";
import { AdminOverviewPage } from "./Overview";
import { AdminProviderDetailPage } from "./ProviderDetail";
import { AdminProvidersListPage } from "./ProvidersList";
import { AdminSourceDetailPage } from "./SourceDetail";
import { AdminSourcesListPage } from "./SourcesList";

function makeRouter(initial: string) {
  const root = createRootRoute({ component: Outlet });
  const layout = createRoute({
    getParentRoute: () => root,
    path: "/admin",
    component: AdminShell,
  });
  const overview = createRoute({
    getParentRoute: () => layout,
    path: "/",
    component: AdminOverviewPage,
  });
  const review = createRoute({
    getParentRoute: () => layout,
    path: "review",
    component: ReviewPage,
  });
  const sourcesList = createRoute({
    getParentRoute: () => layout,
    path: "sources",
    component: AdminSourcesListPage,
  });
  const sourceDetail = createRoute({
    getParentRoute: () => layout,
    path: "sources/$name",
    component: AdminSourceDetailPage,
  });
  const providersList = createRoute({
    getParentRoute: () => layout,
    path: "providers",
    component: AdminProvidersListPage,
  });
  const providerDetail = createRoute({
    getParentRoute: () => layout,
    path: "providers/$id",
    component: AdminProviderDetailPage,
  });
  const download = createRoute({
    getParentRoute: () => layout,
    path: "download",
    component: AdminDownloadPage,
  });
  const codex = createRoute({
    getParentRoute: () => layout,
    path: "codex",
    component: AdminCodexPage,
  });
  const metrics = createRoute({
    getParentRoute: () => layout,
    path: "metrics",
    component: AdminMetricsPage,
  });
  const idMaps = createRoute({
    getParentRoute: () => layout,
    path: "id-maps",
    component: AdminIdMapsPage,
  });
  const maintenance = createRoute({
    getParentRoute: () => layout,
    path: "maintenance",
    component: AdminMaintenancePage,
  });
  // / is referenced from the overview page; provide a stub route so the
  // router does not blow up resolving Link `to`s.
  const home = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: () => null,
  });
  return createRouter({
    routeTree: root.addChildren([
      home,
      layout.addChildren([
        overview,
        review,
        sourcesList,
        sourceDetail,
        providersList,
        providerDetail,
        download,
        codex,
        metrics,
        idMaps,
        maintenance,
      ]),
    ]),
    history: createMemoryHistory({ initialEntries: [initial] }),
  });
}

function renderAt(path: string) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const router = makeRouter(path);
  return render(
    <MantineProvider>
      <Notifications />
      <QueryClientProvider client={client}>
        {/* biome-ignore lint/suspicious/noExplicitAny: route-tree shape differs between test + prod routers */}
        <RouterProvider router={router as any} />
      </QueryClientProvider>
    </MantineProvider>,
  );
}

describe("admin auth gate", () => {
  beforeEach(() => {
    useAdminAuth.getState().clear();
  });
  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("requires admin token before showing any admin page", async () => {
    renderAt("/admin/sources");
    expect(await screen.findByText(/Admin sign-in/i)).toBeInTheDocument();
    expect(screen.queryByText(/Discovery sources/)).not.toBeInTheDocument();
  });
});

describe("admin overview page", () => {
  beforeEach(() => useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN));
  afterEach(() => useAdminAuth.getState().clear());

  it("renders the all-green strip when no sources are failing", async () => {
    renderAt("/admin");
    expect(
      await screen.findByTestId("overview-all-green", undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
  });

  it("renders quick stat cards linking into the detail pages", async () => {
    renderAt("/admin");
    expect(
      await screen.findByTestId("overview-stat-catalog", undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    expect(
      await screen.findByTestId("overview-stat-review-queue"),
    ).toBeInTheDocument();
    expect(
      await screen.findByTestId("overview-stat-id-maps"),
    ).toBeInTheDocument();
  });
});

describe("admin sources page", () => {
  beforeEach(() => useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN));
  afterEach(() => useAdminAuth.getState().clear());

  it("renders the configured source cards", async () => {
    renderAt("/admin/sources");
    expect(
      await screen.findByText(/Discovery sources/, undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    expect(
      await screen.findByTestId("source-card-english-manga-trusted"),
    ).toBeInTheDocument();
  });

  it("dispatches a per-source trigger", async () => {
    renderAt("/admin/sources");
    const button = await screen.findByTestId(
      "poll-english-manga-trusted",
      undefined,
      { timeout: 3000 },
    );
    fireEvent.click(button);
    await waitFor(() => {
      expect(
        screen.getByText(/english-manga-trusted: triggered/),
      ).toBeInTheDocument();
    });
  });

  it("dispatches a backfill after confirming a page count", async () => {
    renderAt("/admin/sources");
    const open = await screen.findByTestId(
      "backfill-english-manga-trusted",
      undefined,
      { timeout: 3000 },
    );
    fireEvent.click(open);
    const pages = await screen.findByTestId(
      "backfill-pages-english-manga-trusted",
    );
    fireEvent.change(pages, { target: { value: "7" } });
    fireEvent.click(
      screen.getByTestId("backfill-confirm-english-manga-trusted"),
    );
    await waitFor(() => {
      expect(
        screen.getByText(/english-manga-trusted: backfill started \(7 pages\)/),
      ).toBeInTheDocument();
    });
  });

  it("renders the running pill from the DTO before any SSE event arrives", async () => {
    // The MSW fixture seeds a second source (`running-on-load`) with
    // `inFlight` set. Hitting the page is the moral equivalent of a
    // hard refresh: SSE map is empty, but the DTO carries the in-flight
    // marker, so the pill must render.
    renderAt("/admin/sources");
    expect(
      await screen.findByTestId("job-pill-source-running-on-load", undefined, {
        timeout: 3000,
      }),
    ).toHaveTextContent(/running/i);
    // The first source has no `inFlight` and no SSE event yet → no pill.
    expect(
      screen.queryByTestId("job-pill-source-english-manga-trusted"),
    ).not.toBeInTheDocument();
  });

  it("renders numeric progress from the DTO inFlight.progress payload", async () => {
    // Third fixture source carries `inFlight.progress = { current: 47, total: 200 }`.
    // Pill should render the fraction without waiting for SSE.
    renderAt("/admin/sources");
    expect(
      await screen.findByTestId(
        "job-pill-source-running-with-progress",
        undefined,
        { timeout: 3000 },
      ),
    ).toHaveTextContent(/47 \/ 200/);
  });

  it("updates the pill from an SSE Progress frame, preferring the frame's payload", async () => {
    renderAt("/admin/sources");
    await screen.findByTestId("source-card-english-manga-trusted", undefined, {
      timeout: 3000,
    });
    const MockES = (
      globalThis as unknown as {
        __mockEventSources: {
          instances: {
            emit: (data: unknown) => void;
            url: string;
            readyState: number;
          }[];
        };
      }
    ).__mockEventSources;
    const live = MockES.instances
      .slice()
      .reverse()
      .find((i) => i.url.includes("/events/jobs") && i.readyState !== 2);
    const es = live;
    expect(es).toBeDefined();
    es?.emit({
      kind: "source",
      id: "english-manga-trusted",
      phase: "progress",
      at: Date.now(),
      progress: { current: 12, total: 75, phase: "enriching" },
    });
    expect(
      await screen.findByTestId(
        "job-pill-source-english-manga-trusted",
        undefined,
        { timeout: 1500 },
      ),
    ).toHaveTextContent(/12 \/ 75/);
  });

  it("flips the source-card pill when a synthetic SSE event arrives", async () => {
    renderAt("/admin/sources");
    await screen.findByTestId("source-card-english-manga-trusted", undefined, {
      timeout: 3000,
    });
    // The EventSource our hook opens is captured by the test setup's
    // mock. Tests share the static `instances` array across runs, so
    // pick the most recent matching instance (the one this render
    // actually subscribed to).
    const MockES = (
      globalThis as unknown as {
        __mockEventSources: {
          instances: {
            emit: (data: unknown) => void;
            url: string;
            readyState: number;
          }[];
        };
      }
    ).__mockEventSources;
    const live = MockES.instances
      .slice()
      .reverse()
      .find((i) => i.url.includes("/events/jobs") && i.readyState !== 2);
    const es = live;
    expect(es).toBeDefined();
    es?.emit({
      kind: "source",
      id: "english-manga-trusted",
      phase: "started",
      at: Date.now(),
    });
    expect(
      await screen.findByTestId(
        "job-pill-source-english-manga-trusted",
        undefined,
        { timeout: 1500 },
      ),
    ).toHaveTextContent(/running/i);
    es?.emit({
      kind: "source",
      id: "english-manga-trusted",
      phase: "finished",
      at: Date.now(),
      result: { triggered: true, skipped: false },
    });
    await waitFor(() => {
      expect(
        screen.getByTestId("job-pill-source-english-manga-trusted"),
      ).toHaveTextContent(/done/i);
    });
  });
});

describe("admin source detail recent runs", () => {
  beforeEach(() => {
    resetSourceRuns();
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
  });

  it("renders the per-run timeline with counts and the failure message", async () => {
    renderAt("/admin/sources/english-manga-trusted");
    expect(await screen.findByTestId("source-recent-runs")).toBeInTheDocument();
    // Default mock rows: a manual failure (newest) and a cron success.
    const failed = screen.getByTestId("source-run-2");
    expect(failed).toHaveTextContent("failed");
    expect(failed).toHaveTextContent("via manual");
    expect(failed).toHaveTextContent("nyaa.si timed out after 30s");
    const ok = screen.getByTestId("source-run-1");
    expect(ok).toHaveTextContent("success");
    expect(ok).toHaveTextContent("75 fetched · 4 new · 3 resolved");
    expect(ok).toHaveTextContent("11.7s");
  });

  it("hides the timeline when the source has no recorded runs", async () => {
    seedSourceRuns("english-manga-trusted", []);
    renderAt("/admin/sources/english-manga-trusted");
    // Wait for the page to settle (config block renders), then assert.
    await screen.findByText("Config");
    expect(screen.queryByTestId("source-recent-runs")).not.toBeInTheDocument();
  });
});

describe("admin providers page", () => {
  beforeEach(() => useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN));
  afterEach(() => useAdminAuth.getState().clear());

  it("renders provider cards with the renamed 'Refresh cache' button", async () => {
    renderAt("/admin/providers");
    expect(
      await screen.findByTestId("provider-card-mangabaka", undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    expect(
      await screen.findByTestId("refresh-mangabaka", undefined, {
        timeout: 3000,
      }),
    ).toHaveTextContent(/refresh cache/i);
  });

  it("never renders the raw api_key value", async () => {
    const { container } = renderAt("/admin/providers");
    await screen.findByTestId("provider-card-mangabaka", undefined, {
      timeout: 3000,
    });
    expect(container.innerHTML).not.toMatch(/"apiKey"\s*:\s*"/);
    expect(screen.getByTestId("api-key-set-badge")).toHaveTextContent(/set/i);
  });

  it("dispatches the per-provider cache-refresh mutation with the renamed copy", async () => {
    renderAt("/admin/providers");
    const button = await screen.findByTestId("refresh-mangabaka", undefined, {
      timeout: 3000,
    });
    fireEvent.click(button);
    await waitFor(() => {
      expect(
        screen.getByText(/mangabaka: cache refresh triggered/),
      ).toBeInTheDocument();
    });
  });
});

describe("admin download page", () => {
  beforeEach(() => useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN));
  afterEach(() => useAdminAuth.getState().clear());

  it("exposes a Download entry in the admin nav", async () => {
    renderAt("/admin/download");
    expect(
      await screen.findByTestId("admin-nav-download", undefined, {
        timeout: 3000,
      }),
    ).toHaveTextContent(/download/i);
  });

  it("renders the client card with connection info and reachable badge", async () => {
    renderAt("/admin/download");
    expect(
      await screen.findByTestId("download-card", undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("download-reachable")).toHaveTextContent(
      /reachable/i,
    );
  });
});

describe("admin codex page", () => {
  beforeEach(() => useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN));
  afterEach(() => useAdminAuth.getState().clear());

  it("exposes a Codex entry in the admin nav and renders the card", async () => {
    renderAt("/admin/codex");
    expect(
      await screen.findByTestId("admin-nav-codex", undefined, {
        timeout: 3000,
      }),
    ).toHaveTextContent(/codex/i);
    expect(await screen.findByTestId("codex-card")).toBeInTheDocument();
  });

  it("renders the per-sweep refresh history with counts and failures", async () => {
    renderAt("/admin/codex");
    expect(
      await screen.findByTestId("codex-recent-sync-runs", undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    expect(screen.getByText("Recent syncs")).toBeInTheDocument();
    // Success row shows the linked/fetched counts.
    expect(screen.getByText(/2 of 18 linked/)).toBeInTheDocument();
    // Failed sweep surfaces its error.
    expect(screen.getByText(/api_key rejected \(401\)/)).toBeInTheDocument();
  });

  it("tests the codex connection and toasts the unreachable result", async () => {
    renderAt("/admin/codex");
    const btn = await screen.findByTestId("codex-test-button", undefined, {
      timeout: 3000,
    });
    fireEvent.click(btn);
    await waitFor(() => {
      expect(
        screen.getByText(/Unreachable: connection refused/),
      ).toBeInTheDocument();
    });
  });
});

describe("admin metrics page", () => {
  beforeEach(() => useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN));
  afterEach(() => useAdminAuth.getState().clear());

  it("renders per-source metrics card with sparkline and outcome breakdown", async () => {
    renderAt("/admin/metrics");
    expect(
      await screen.findByTestId(
        "metrics-card-english-manga-trusted",
        undefined,
        { timeout: 3000 },
      ),
    ).toBeInTheDocument();
    expect(
      await screen.findByTestId("metrics-sparkline", undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    expect(
      await screen.findByTestId("outcome-breakdown", undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    expect(screen.getByText(/fetch max/i)).toBeInTheDocument();
    expect(screen.getByText(/ttr p95/i)).toBeInTheDocument();
    expect(screen.getByTestId("review-queue-metrics-card")).toBeInTheDocument();
    expect(
      screen.getByTestId("review-queue-depth-sparkline"),
    ).toBeInTheDocument();
  });
});

describe("admin maintenance page", () => {
  beforeEach(() => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    // Mantine's notification store is a module-level singleton; prior
    // describes (sources, providers, ...) leave their notifications in
    // it and the default limit=5 makes later assertions miss new ones.
    notifications.clean();
  });
  afterEach(() => useAdminAuth.getState().clear());

  it("renders the invalidation card", async () => {
    renderAt("/admin/maintenance");
    expect(
      await screen.findByTestId("maintenance-invalidate-card", undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("maintenance-invalidate-button"),
    ).toHaveTextContent(/invalidate metadata hashes/i);
  });

  it("opens a confirm modal before calling the API", async () => {
    renderAt("/admin/maintenance");
    const open = await screen.findByTestId(
      "maintenance-invalidate-button",
      undefined,
      { timeout: 3000 },
    );
    fireEvent.click(open);
    // The confirm button only exists once the modal is open; finding it
    // proves the click opened the modal rather than firing the request.
    expect(
      await screen.findByTestId("maintenance-invalidate-confirm"),
    ).toBeInTheDocument();
  });

  it("dispatches the invalidation mutation and surfaces the counts", async () => {
    renderAt("/admin/maintenance");
    const open = await screen.findByTestId(
      "maintenance-invalidate-button",
      undefined,
      { timeout: 3000 },
    );
    fireEvent.click(open);
    const confirm = await screen.findByTestId("maintenance-invalidate-confirm");
    fireEvent.click(confirm);
    // MSW handler returns invalidated=12, skippedManual=1.
    await waitFor(() => {
      expect(
        screen.getByText(/12 cleared, 1 manual row\(s\) left alone/i),
      ).toBeInTheDocument();
    });
  });

  it("renders the refresh card with stale + all buttons", async () => {
    renderAt("/admin/maintenance");
    expect(
      await screen.findByTestId("maintenance-refresh-card", undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("maintenance-refresh-button")).toHaveTextContent(
      /refresh stale series/i,
    );
    expect(
      screen.getByTestId("maintenance-refresh-all-button"),
    ).toHaveTextContent(/refresh all series/i);
  });

  it("dispatches the stale refresh mutation and surfaces the result", async () => {
    renderAt("/admin/maintenance");
    const button = await screen.findByTestId(
      "maintenance-refresh-button",
      undefined,
      { timeout: 3000 },
    );
    fireEvent.click(button);
    // MSW handler returns provider=mangabaka, batchSize=25, minAgeDays=7.
    await waitFor(() => {
      expect(
        screen.getByText(/mangabaka: up to 25 row\(s\), min age 7d/i),
      ).toBeInTheDocument();
    });
  });

  it("dispatches the full drain mutation when ALL is clicked", async () => {
    renderAt("/admin/maintenance");
    const button = await screen.findByTestId(
      "maintenance-refresh-all-button",
      undefined,
      { timeout: 3000 },
    );
    fireEvent.click(button);
    // MSW handler echoes scope=all when ?all=true.
    await waitFor(() => {
      expect(
        screen.getByText(/draining every eligible row in batches of 25/i),
      ).toBeInTheDocument();
    });
  });

  it("dispatches the recompute-spans mutation and surfaces the counts", async () => {
    renderAt("/admin/maintenance");
    const button = await screen.findByTestId(
      "maintenance-recompute-button",
      undefined,
      { timeout: 3000 },
    );
    fireEvent.click(button);
    // MSW handler returns releasesRewritten=18, seriesUpdated=5.
    await waitFor(() => {
      expect(
        screen.getByText(
          /18 release span\(s\) rewritten, 5 series mark\(s\) updated/i,
        ),
      ).toBeInTheDocument();
    });
  });

  it("renders the re-enrich card with the default review statuses", async () => {
    renderAt("/admin/maintenance");
    expect(
      await screen.findByTestId("maintenance-reenrich-card", undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("maintenance-reenrich-button")).toHaveTextContent(
      /re-enrich releases/i,
    );
  });

  it("dispatches the re-enrich mutation with the selected source + statuses", async () => {
    renderAt("/admin/maintenance");
    const button = await screen.findByTestId(
      "maintenance-reenrich-button",
      undefined,
      { timeout: 3000 },
    );
    // The button stays disabled until the /sources query resolves and the
    // source select defaults to the first item.
    await waitFor(() => expect(button).not.toBeDisabled());
    fireEvent.click(button);
    // Source defaults to the first /sources item; statuses default to the
    // review set. The MSW handler echoes them back.
    await waitFor(() => {
      expect(
        screen.getByText(
          /english-manga-trusted: unresolved, ambiguous, review_pending/i,
        ),
      ).toBeInTheDocument();
    });
  });

  it("re-enriches every source after Select all", async () => {
    renderAt("/admin/maintenance");
    const button = await screen.findByTestId(
      "maintenance-reenrich-button",
      undefined,
      { timeout: 3000 },
    );
    await waitFor(() => expect(button).not.toBeDisabled());
    fireEvent.click(screen.getByTestId("reenrich-source-all"));
    fireEvent.click(button);
    // Fan-out dispatches one request per source; the triggered notification
    // lists every source name (the MSW handler echoes each back).
    await waitFor(() => {
      expect(
        screen.getByText(
          /english-manga-trusted, running-on-load, running-with-progress: unresolved, ambiguous, review_pending/i,
        ),
      ).toBeInTheDocument();
    });
  });

  it("renders the invalidate-covers card", async () => {
    renderAt("/admin/maintenance");
    expect(
      await screen.findByTestId(
        "maintenance-invalidate-covers-card",
        undefined,
        { timeout: 3000 },
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("maintenance-invalidate-covers-button"),
    ).toHaveTextContent(/invalidate cover cache/i);
  });

  it("dispatches the invalidate-covers mutation and surfaces the counts", async () => {
    renderAt("/admin/maintenance");
    const open = await screen.findByTestId(
      "maintenance-invalidate-covers-button",
      undefined,
      { timeout: 3000 },
    );
    fireEvent.click(open);
    const confirm = await screen.findByTestId(
      "maintenance-invalidate-covers-confirm",
    );
    fireEvent.click(confirm);
    // MSW handler returns filesDeleted=7, bytesFreed=245678.
    await waitFor(() => {
      expect(
        screen.getByText(/7 file\(s\) deleted, 239\.9 KB freed/i),
      ).toBeInTheDocument();
    });
  });
});

describe("admin section-nav mobile drawer", () => {
  beforeEach(() => useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN));
  afterEach(() => useAdminAuth.getState().clear());

  it("hides the section links behind a burger until opened", async () => {
    renderAt("/admin");
    // The mobile drawer mounts its links only when open.
    expect(
      screen.queryByTestId("admin-nav-mobile-sources"),
    ).not.toBeInTheDocument();
    fireEvent.click(
      await screen.findByRole("button", { name: /open admin sections/i }),
    );
    expect(
      await screen.findByTestId("admin-nav-mobile-sources"),
    ).toBeInTheDocument();
  });

  it("closes the drawer after tapping a section", async () => {
    renderAt("/admin");
    fireEvent.click(
      await screen.findByRole("button", { name: /open admin sections/i }),
    );
    const link = await screen.findByTestId("admin-nav-mobile-sources");
    fireEvent.click(link);
    // Navigating collapses the drawer, unmounting its links.
    await waitFor(() =>
      expect(
        screen.queryByTestId("admin-nav-mobile-sources"),
      ).not.toBeInTheDocument(),
    );
  });

  it("keeps the desktop rail links present and unambiguous", async () => {
    renderAt("/admin");
    // Rail link is always mounted; only one element carries the rail testid.
    expect(await screen.findByTestId("admin-nav-sources")).toBeInTheDocument();
  });
});

describe("admin id-maps page", () => {
  beforeEach(() => useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN));
  afterEach(() => useAdminAuth.getState().clear());

  it("renders provider row counts and MU redirect cache stats", async () => {
    renderAt("/admin/id-maps");
    expect(
      await screen.findByTestId("admin-id-maps", undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
    expect(
      await screen.findByTestId("id-map-row-mangaupdates"),
    ).toBeInTheDocument();
    // Both labels appear on the page; "tombstones" also shows up in
    // the descriptive prose, so allow N≥1 matches rather than asserting
    // uniqueness.
    expect(screen.getAllByText(/modern slugs/i).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/tombstones/i).length).toBeGreaterThan(0);
  });
});
