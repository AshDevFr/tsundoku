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
import { useUiPrefs } from "@/stores/uiPrefs";
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

describe("AdminSourcesListPage import by url", () => {
  beforeEach(() => {
    resetSearch();
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
  });
  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("imports a pasted post url and reports the resolution outcome", async () => {
    renderPage();

    const input = await screen.findByTestId("import-release-url");
    fireEvent.change(input, {
      target: { value: "https://nyaa.si/view/2111533" },
    });
    fireEvent.click(screen.getByTestId("import-release-submit"));

    const result = await screen.findByTestId("import-release-result");
    expect(result).toHaveTextContent("ReZero - Starting Life in Another World");
    expect(result).toHaveTextContent("resolved");
  });

  it("flags a url the catalog already holds", async () => {
    renderPage();

    const input = await screen.findByTestId("import-release-url");
    fireEvent.change(input, {
      target: { value: "https://nyaa.si/view/known" },
    });
    fireEvent.click(screen.getByTestId("import-release-submit"));

    expect(
      await screen.findByTestId("import-release-result"),
    ).toHaveTextContent(/already/i);
  });

  it("surfaces the server's message when the url is not recognized", async () => {
    renderPage();

    const input = await screen.findByTestId("import-release-url");
    fireEvent.change(input, {
      target: { value: "https://example.org/view/1" },
    });
    fireEvent.click(screen.getByTestId("import-release-submit"));

    expect(await screen.findByTestId("import-release-error")).toHaveTextContent(
      /recognize/i,
    );
  });

  it("keeps the submit button disabled until a url is entered", async () => {
    renderPage();
    expect(await screen.findByTestId("import-release-submit")).toBeDisabled();
  });

  it("hides the card when no search entries are configured", async () => {
    seedSearchEntries([]);
    renderPage();
    await screen.findByText(/No sources registered/);
    expect(screen.queryByTestId("import-release-card")).not.toBeInTheDocument();
  });
});

/// Like `renderPage`, but leaves the default sources fixture in place so the
/// card grid actually renders.
function renderPageWithSources() {
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

function detailsSwitch(): HTMLInputElement {
  const el = screen.getByTestId("toggle-source-details");
  return (
    el.tagName === "INPUT" ? el : el.querySelector("input")
  ) as HTMLInputElement;
}

describe("AdminSourcesListPage — source card details toggle", () => {
  beforeEach(() => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    // The store persists to localStorage, so reset it between cases or one
    // toggle leaks into every later test.
    useUiPrefs.setState({ sourceCardDetails: false });
  });
  afterEach(() => useAdminAuth.getState().clear());

  // Off by default: the config block is reference material, and twenty-odd
  // cards carrying five rows each is what made the page unusable.
  it("hides card config details by default", async () => {
    renderPageWithSources();
    const card = await screen.findByTestId("source-card-english-manga-trusted");
    expect(card).not.toHaveTextContent("cron");
    // The card itself, and its status line, must still be there.
    expect(card).toHaveTextContent("english-manga-trusted");
    expect(card).toHaveTextContent("75 new releases");
  });

  it("reveals them for every card at once when switched on", async () => {
    renderPageWithSources();
    await screen.findByTestId("source-card-english-manga-trusted");
    fireEvent.click(detailsSwitch());

    await waitFor(() =>
      expect(
        screen.getByTestId("source-card-english-manga-trusted"),
      ).toHaveTextContent("cron"),
    );
    expect(
      screen.getByTestId("source-card-english-manga-trusted"),
    ).toHaveTextContent("timeout");
  });

  it("remembers the choice across remounts", async () => {
    const first = renderPageWithSources();
    await screen.findByTestId("source-card-english-manga-trusted");
    fireEvent.click(detailsSwitch());
    await waitFor(() =>
      expect(useUiPrefs.getState().sourceCardDetails).toBe(true),
    );
    first.unmount();

    renderPageWithSources();
    const card = await screen.findByTestId("source-card-english-manga-trusted");
    expect(card).toHaveTextContent("cron");
  });
});
