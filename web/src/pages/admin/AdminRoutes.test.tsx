import { MantineProvider } from "@mantine/core";
import { Notifications } from "@mantine/notifications";
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
import { ADMIN_TEST_TOKEN } from "@/mocks/handlers";
import { ReviewPage } from "@/pages/ReviewPage";
import { useAdminAuth } from "@/stores/auth";
import { AdminIdMapsPage } from "./IdMaps";
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
        metrics,
        idMaps,
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
