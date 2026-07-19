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
import { render, screen } from "@testing-library/react";
import { HttpResponse, http } from "msw";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  ADMIN_TEST_TOKEN,
  resetSearch,
  seedSearchEntries,
  seedSearchRuns,
} from "@/mocks/handlers";
import { server } from "@/mocks/server";
import { useAdminAuth } from "@/stores/auth";
import { AdminSourcesListPage } from "./SourcesList";

function renderPage() {
  // No sources in these tests (keeps the SourceCard grid out of the way);
  // the page still renders its search sections below the empty state. A
  // minimal router backs the recent-searches series links.
  server.use(
    http.get("/api/v1/sources", () => HttpResponse.json({ items: [] })),
  );
  const root = createRootRoute({ component: Outlet });
  const page = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: AdminSourcesListPage,
  });
  const series = createRoute({
    getParentRoute: () => root,
    path: "/series/$id",
    component: () => null,
  });
  const router = createRouter({
    routeTree: root.addChildren([page, series]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
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

describe("AdminSourcesListPage search endpoints", () => {
  beforeEach(() => {
    resetSearch();
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
  });
  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("lists the configured search endpoints with the default badge", async () => {
    renderPage();
    expect(
      await screen.findByTestId("search-endpoints-section"),
    ).toBeInTheDocument();

    const eng = screen.getByTestId("search-endpoint-Nyaa Literature - Eng");
    expect(eng).toHaveTextContent("default");
    expect(eng).toHaveTextContent("https://nyaa.si/?f=0&c=3_1");
    expect(eng).toHaveTextContent("nyaa");

    const raw = screen.getByTestId("search-endpoint-Nyaa Literature - Raw");
    expect(raw).not.toHaveTextContent("default");
    expect(raw).toHaveTextContent("https://nyaa.si/?f=0&c=3_3");
  });

  it("lists recent searches across series with linked titles", async () => {
    seedSearchRuns([
      {
        id: 3,
        ranAt: Math.floor(Date.now() / 1000) - 600,
        finishedAt: Math.floor(Date.now() / 1000) - 540,
        searchName: "Nyaa Literature - Eng",
        seriesId: 5,
        trigger: "manual",
        outcome: "success",
        queriesAttempted: 2,
        pagesFetched: 3,
        releasesSeen: 30,
        releasesNew: 4,
        error: null,
        seriesTitle: "Jujutsu Kaisen",
      },
      {
        id: 2,
        ranAt: Math.floor(Date.now() / 1000) - 1200,
        finishedAt: null,
        searchName: "Nyaa Literature - Raw",
        seriesId: 1,
        trigger: "cli",
        outcome: "running",
        queriesAttempted: null,
        pagesFetched: null,
        releasesSeen: null,
        releasesNew: null,
        error: null,
        seriesTitle: "Chainsaw Man",
      },
    ]);
    renderPage();
    expect(
      await screen.findByTestId("recent-searches-section"),
    ).toBeInTheDocument();

    const ok = screen.getByTestId("recent-search-3");
    expect(ok).toHaveTextContent("Jujutsu Kaisen");
    expect(ok).toHaveTextContent("Nyaa Literature - Eng");
    expect(ok).toHaveTextContent("success");
    expect(ok).toHaveTextContent("4");
    // The series title links into the catalog.
    const link = ok.querySelector("a");
    expect(link?.getAttribute("href")).toBe("/series/5");

    const running = screen.getByTestId("recent-search-2");
    expect(running).toHaveTextContent("Chainsaw Man");
    expect(running).toHaveTextContent("running…");
    expect(running).toHaveTextContent("—");
  });

  it("hides recent searches when nothing has run", async () => {
    renderPage();
    await screen.findByText(/No sources registered/);
    expect(
      screen.queryByTestId("recent-searches-section"),
    ).not.toBeInTheDocument();
  });

  it("hides the section when no search entries are configured", async () => {
    seedSearchEntries([]);
    renderPage();
    // Wait for the page to settle (sources empty-state renders), then
    // assert the section never appeared.
    await screen.findByText(/No sources registered/);
    expect(
      screen.queryByTestId("search-endpoints-section"),
    ).not.toBeInTheDocument();
  });
});
