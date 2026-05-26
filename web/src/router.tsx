import {
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
} from "@tanstack/react-router";
import { AppShell } from "@/components/AppShell";
import { FeedPage } from "@/pages/FeedPage";
import { ReviewPage } from "@/pages/ReviewPage";
import { SeriesDetailPage } from "@/pages/SeriesDetailPage";
import type { FilterSearch } from "@/stores/filters";

// Code-based routing keeps the scaffold self-contained (no router codegen
// plugin). Switch to file-based routing later if the route tree grows.
const rootRoute = createRootRoute({
  component: () => (
    <AppShell>
      <Outlet />
    </AppShell>
  ),
});

export const feedRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: FeedPage,
  validateSearch: (raw: Record<string, unknown>): FilterSearch => {
    const search: FilterSearch = {};
    if (typeof raw.kind === "string" && raw.kind) search.kind = raw.kind;
    if (typeof raw.status === "string" && raw.status)
      search.status = raw.status;
    if (typeof raw.sort === "string" && raw.sort) search.sort = raw.sort;
    if (typeof raw.order === "string" && raw.order) search.order = raw.order;
    if (raw.owned === true || raw.owned === "true") search.owned = true;
    else if (raw.owned === false || raw.owned === "false") search.owned = false;
    const page = Number(raw.page);
    if (Number.isFinite(page) && page > 0) search.page = Math.floor(page);
    return search;
  },
});

export const seriesDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/series/$id",
  component: SeriesDetailPage,
});

export const reviewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/review",
  component: ReviewPage,
});

const routeTree = rootRoute.addChildren([
  feedRoute,
  seriesDetailRoute,
  reviewRoute,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
