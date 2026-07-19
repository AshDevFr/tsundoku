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
  resetReviewQueue,
  resetSearch,
  resetSeries,
  seedSearchEntries,
  seedSearchRuns,
  setSearchBusy,
} from "@/mocks/handlers";
import { server } from "@/mocks/server";
import { useAdminAuth } from "@/stores/auth";
import { SeriesDetailPage } from "./SeriesDetailPage";

// Rebuild the `/series/$id` route locally. `SeriesDetailPage` reads its params
// via the prod `seriesDetailRoute`, whose id ("/series/$id") matches the path
// below, so `useParams()` resolves against this match. `initialEntry` lets a
// test seed the URL (including the feed filter query string the detail route
// carries through).
function renderSeriesDetail(id: number, initialEntry = `/series/${id}`) {
  const root = createRootRoute({ component: Outlet });
  // A stub feed route so the "Back to feed" Link (`to="/"`) resolves and its
  // href reflects the carried-through search params.
  const feed = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: () => null,
  });
  const series = createRoute({
    getParentRoute: () => root,
    path: "/series/$id",
    component: SeriesDetailPage,
    // Mirror prod: pass through whatever filter params arrive on the URL.
    validateSearch: (raw: Record<string, unknown>) => raw,
  });
  const router = createRouter({
    routeTree: root.addChildren([feed, series]),
    history: createMemoryHistory({ initialEntries: [initialEntry] }),
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = render(
    <MantineProvider>
      <Notifications />
      <QueryClientProvider client={client}>
        {/* biome-ignore lint/suspicious/noExplicitAny: route-tree shape differs between test + prod routers */}
        <RouterProvider router={router as any} />
      </QueryClientProvider>
    </MantineProvider>,
  );
  return { ...view, router };
}

describe("SeriesDetailPage", () => {
  beforeEach(() => {
    resetReviewQueue();
    resetSeries();
    resetSearch();
    useAdminAuth.getState().clear();
  });

  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("renders both genres and tags from the series metadata", async () => {
    renderSeriesDetail(1);
    // Series 1 (Chainsaw Man) carries genres and tags in the mock.
    await screen.findByText("Chainsaw Man");
    // Genres.
    expect(screen.getByText("action")).toBeInTheDocument();
    expect(screen.getByText("horror")).toBeInTheDocument();
    // Tags.
    expect(screen.getByText("devil hunter")).toBeInTheDocument();
    expect(screen.getByText("gore")).toBeInTheDocument();
  });

  it("carries the feed filters into the Back to feed link", async () => {
    renderSeriesDetail(
      1,
      "/series/1?kind=manga&genres=action&page=3&view=list",
    );
    const back = await screen.findByRole("link", { name: /Back to feed/ });
    const href = back.getAttribute("href") ?? "";
    expect(href).toMatch(/^\/\?/); // links to the feed, not just "/series/1"
    expect(href).toContain("kind=manga");
    expect(href).toContain("genres=action");
    expect(href).toContain("page=3");
    expect(href).toContain("view=list");
  });

  it("hides the Move action when no admin token is present", async () => {
    renderSeriesDetail(1);
    await screen.findByText(/Chainsaw Man v01/);
    expect(
      screen.queryByTestId("move-release-nyaa:111"),
    ).not.toBeInTheDocument();
  });

  it("opens the Move modal with catalog + provider search once authed", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderSeriesDetail(1);
    const moveBtn = await screen.findByTestId("move-release-nyaa:111");
    fireEvent.click(moveBtn);

    const dialog = await screen.findByRole("dialog");
    // Defaults to the catalog tab: the catalog search input is mounted.
    await waitFor(() => {
      if (!dialog.querySelector('[data-testid="link-existing-search"]')) {
        throw new Error("catalog search input not rendered");
      }
    });
  });

  it("hides the Edit button without an admin token", async () => {
    // Series 10 is manual, but editing is admin-only.
    renderSeriesDetail(10);
    await screen.findByText("Obscure Doujin Anthology");
    expect(screen.queryByTestId("edit-series")).not.toBeInTheDocument();
  });

  it("hides the Edit button for a provider-backed series even as admin", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    // Series 1 (Chainsaw Man) is metadataSource="offline_cache".
    renderSeriesDetail(1);
    await screen.findByText("Chainsaw Man");
    expect(screen.queryByTestId("edit-series")).not.toBeInTheDocument();
  });

  it("hides the ignore-completion toggle without an admin token", async () => {
    // Series 5 is owned on Codex, but the control is admin-only.
    renderSeriesDetail(5);
    await screen.findByText("Jujutsu Kaisen");
    expect(
      screen.queryByTestId("toggle-ignore-completion"),
    ).not.toBeInTheDocument();
  });

  it("toggles completion tracking on a provider-backed owned series", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    // Series 5 (Jujutsu Kaisen) is provider-backed and owned with status
    // "behind" — the manual-edit PATCH would reject it, but this toggle works.
    renderSeriesDetail(5);
    await screen.findByTestId("codex-badge-behind");
    const toggle = await screen.findByTestId("toggle-ignore-completion");
    expect(toggle).toHaveTextContent("Ignore completion");

    fireEvent.click(toggle);

    // The mutation invalidates + refetches; the badge flips to "tracking off"
    // and the button now offers to resume.
    await screen.findByTestId("codex-badge-ignored");
    expect(
      await screen.findByTestId("toggle-ignore-completion"),
    ).toHaveTextContent("Resume tracking");
  });

  it("hides the wishlist toggle without an admin token", async () => {
    renderSeriesDetail(1);
    await screen.findByText("Chainsaw Man");
    expect(screen.queryByTestId("toggle-wishlist")).not.toBeInTheDocument();
  });

  it("clips a series to the wishlist and reflects the new state", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderSeriesDetail(1);
    await screen.findByText("Chainsaw Man");
    const toggle = await screen.findByTestId("toggle-wishlist");
    expect(toggle).toHaveTextContent("Add to wishlist");

    fireEvent.click(toggle);

    // The mutation invalidates + refetches; the button flips to "on wishlist".
    await waitFor(() =>
      expect(screen.getByTestId("toggle-wishlist")).toHaveTextContent(
        "On wishlist",
      ),
    );
  });

  it("edits a manual series and reflects the change in the detail view", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderSeriesDetail(10);
    const editBtn = await screen.findByTestId("edit-series");
    fireEvent.click(editBtn);

    const dialog = await screen.findByRole("dialog");
    const titleInput = dialog.querySelector<HTMLInputElement>(
      '[data-testid="edit-series-title"]',
    );
    if (!titleInput) throw new Error("title input not rendered");
    fireEvent.change(titleInput, {
      target: { value: "Renamed Anthology" },
    });
    fireEvent.click(screen.getByTestId("edit-series-submit"));

    // The detail view picks up the new title after the mutation invalidates
    // and refetches the detail query.
    await screen.findByText("Renamed Anthology");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("disables save when the title is cleared", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderSeriesDetail(10);
    fireEvent.click(await screen.findByTestId("edit-series"));
    const dialog = await screen.findByRole("dialog");
    const titleInput = dialog.querySelector<HTMLInputElement>(
      '[data-testid="edit-series-title"]',
    );
    if (!titleInput) throw new Error("title input not rendered");
    fireEvent.change(titleInput, { target: { value: "  " } });
    expect(screen.getByTestId("edit-series-submit")).toBeDisabled();
  });

  it("offers a Nyaa search link scoped to the series title", async () => {
    renderSeriesDetail(1);
    const link = await screen.findByTestId("search-nyaa");
    const href = link.getAttribute("href") ?? "";
    expect(href).toContain("https://nyaa.si/?f=0&c=3_1&q=");
    expect(href).toContain(encodeURIComponent("Chainsaw Man"));
    expect(link).toHaveAttribute("target", "_blank");
  });

  it("navigates to the feed filtered by a clicked genre badge", async () => {
    const { router } = renderSeriesDetail(1);
    fireEvent.click(await screen.findByTestId("genre-badge-action"));
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/");
    });
    expect(router.state.location.search).toMatchObject({
      genres: ["action"],
      genresMode: "any",
      page: 1,
    });
  });

  it("navigates to the feed filtered by a clicked tag badge", async () => {
    const { router } = renderSeriesDetail(1);
    fireEvent.click(await screen.findByTestId("tag-badge-gore"));
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/");
    });
    expect(router.state.location.search).toMatchObject({
      tags: ["gore"],
      tagsMode: "any",
      page: 1,
    });
  });

  it("hides bulk-select checkboxes without an admin token", async () => {
    renderSeriesDetail(1);
    await screen.findByText(/Chainsaw Man v01/);
    expect(
      screen.queryByTestId("select-release-nyaa:111"),
    ).not.toBeInTheDocument();
  });

  it("keeps already-sent releases selectable for re-send", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderSeriesDetail(1);
    // v03 was already sent, but it could have been cancelled/lost in the
    // client, so it still gets a checkbox (re-send is allowed).
    await screen.findByTestId("select-release-nyaa:111");
    await screen.findByTestId("select-release-nyaa:112");
    expect(screen.getByTestId("select-release-nyaa:113")).toBeInTheDocument();
  });

  it("shift-clicking selects the full span including already-sent releases", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderSeriesDetail(1);

    // Anchor on v01, then shift-click v04: the range covers v01, v02, v03, v04
    // — the already-sent v03 is included now, so 4 end up selected.
    fireEvent.click(await screen.findByTestId("select-release-nyaa:111"));
    fireEvent.click(await screen.findByTestId("select-release-nyaa:114"), {
      shiftKey: true,
    });

    const sendBtn = await screen.findByTestId("bulk-send");
    expect(sendBtn).toHaveTextContent("Send 4 to client");
    expect(
      (screen.getByTestId("select-release-nyaa:113") as HTMLInputElement)
        .checked,
    ).toBe(true);
  });

  it("bulk-sends selected releases and reports an aggregated result", async () => {
    // One id succeeds, the other fails, so the toast tallies both.
    server.use(
      http.post("/api/v1/releases/:id/send-to-client", ({ params }) => {
        if (String(params.id) === "nyaa:112") {
          return new HttpResponse(
            JSON.stringify({ error: "send_failed", message: "boom" }),
            { status: 500, headers: { "content-type": "application/json" } },
          );
        }
        return HttpResponse.json({ id: String(params.id) });
      }),
    );
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderSeriesDetail(1);

    fireEvent.click(await screen.findByTestId("select-release-nyaa:111"));
    fireEvent.click(await screen.findByTestId("select-release-nyaa:112"));

    const sendBtn = await screen.findByTestId("bulk-send");
    expect(sendBtn).toHaveTextContent("Send 2 to client");
    fireEvent.click(sendBtn);

    expect(
      await screen.findByText(/1 sent, 1 failed/, undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
  });

  it("hides the search-releases button for anon sessions", async () => {
    renderSeriesDetail(1);
    await screen.findByText("Chainsaw Man");
    expect(screen.queryByTestId("search-releases")).not.toBeInTheDocument();
  });

  it("hides the search-releases button when no entries are configured", async () => {
    seedSearchEntries([]);
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderSeriesDetail(1);
    await screen.findByText("Chainsaw Man");
    // Give the entries query a beat to resolve empty; the button must not
    // appear afterwards either.
    await waitFor(() => {
      expect(screen.queryByTestId("search-releases")).not.toBeInTheDocument();
    });
  });

  // Full happy path exercises the ~2s poll cycle on top of the mock's
  // simulated walk, so it legitimately needs more than vitest's default
  // 5s timeout on slow CI runners.
  it("runs a search and notifies with the new-release count", {
    timeout: 15_000,
  }, async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderSeriesDetail(1);
    await screen.findByText("Chainsaw Man");

    const btn = await screen.findByTestId("search-releases");
    fireEvent.click(btn);

    // Mock walk completes after ~700ms; the 2s poll then notices.
    expect(
      await screen.findByText(/Search found 3 new releases/, undefined, {
        timeout: 6000,
      }),
    ).toBeInTheDocument();
  });

  it("shows an already-running notice when the entry is busy", async () => {
    setSearchBusy(true);
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderSeriesDetail(1);
    await screen.findByText("Chainsaw Man");

    fireEvent.click(await screen.findByTestId("search-releases"));
    expect(
      await screen.findByText(/already running/, undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
  });

  it("shows the search history popover when the series has past runs", async () => {
    seedSearchRuns([
      {
        id: 7,
        ranAt: Math.floor(Date.now() / 1000) - 3600,
        finishedAt: Math.floor(Date.now() / 1000) - 3540,
        searchName: "Nyaa Literature - Eng",
        seriesId: 1,
        trigger: "manual",
        outcome: "success",
        queriesAttempted: 3,
        pagesFetched: 4,
        releasesSeen: 41,
        releasesNew: 5,
        error: null,
      },
      {
        id: 6,
        ranAt: Math.floor(Date.now() / 1000) - 7200,
        finishedAt: Math.floor(Date.now() / 1000) - 7100,
        searchName: "Nyaa Literature - Raw",
        seriesId: 1,
        trigger: "cli",
        outcome: "error",
        queriesAttempted: 1,
        pagesFetched: 0,
        releasesSeen: 0,
        releasesNew: 0,
        error: "nyaa unreachable",
      },
    ]);
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderSeriesDetail(1);
    await screen.findByText("Chainsaw Man");

    fireEvent.click(await screen.findByTestId("search-history"));
    const ok = await screen.findByTestId("search-history-run-7");
    expect(ok).toHaveTextContent("Nyaa Literature - Eng");
    expect(ok).toHaveTextContent("41 hits → 5 new · via manual");
    const failed = screen.getByTestId("search-history-run-6");
    expect(failed).toHaveTextContent("failed");
    expect(failed).toHaveTextContent("nyaa unreachable");
  });

  it("hides the search history affordance when the series has no runs", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderSeriesDetail(1);
    await screen.findByText("Chainsaw Man");
    await screen.findByTestId("search-releases");
    expect(screen.queryByTestId("search-history")).not.toBeInTheDocument();
  });

  it("lets the dropdown pick a non-default entry", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    // Capture the trigger body to prove the picked entry is sent.
    let sentBody: unknown;
    server.use(
      http.post(
        "/api/v1/series/:id/search-releases",
        async ({ request, params }) => {
          sentBody = await request.json();
          return HttpResponse.json({
            search: "Nyaa Literature - Raw",
            seriesId: Number(params.id),
            triggered: false,
            skipped: true,
          });
        },
      ),
    );
    renderSeriesDetail(1);
    await screen.findByText("Chainsaw Man");

    fireEvent.click(await screen.findByTestId("search-releases-options"));
    fireEvent.click(
      await screen.findByTestId("search-releases-entry-Nyaa Literature - Raw"),
    );

    await waitFor(() => {
      expect(sentBody).toEqual({ search: "Nyaa Literature - Raw" });
    });
  });
});
