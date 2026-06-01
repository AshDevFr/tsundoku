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
import { ADMIN_TEST_TOKEN, resetReviewQueue } from "@/mocks/handlers";
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

describe("SeriesDetailPage", () => {
  beforeEach(() => {
    resetReviewQueue();
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
});
