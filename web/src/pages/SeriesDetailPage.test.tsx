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
// below, so `useParams()` resolves against this match.
function renderSeriesDetail(id: number) {
  const root = createRootRoute({ component: Outlet });
  const series = createRoute({
    getParentRoute: () => root,
    path: "/series/$id",
    component: SeriesDetailPage,
  });
  const router = createRouter({
    routeTree: root.addChildren([series]),
    history: createMemoryHistory({ initialEntries: [`/series/${id}`] }),
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
});
