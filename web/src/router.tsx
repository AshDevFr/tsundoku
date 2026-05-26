import {
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
} from "@tanstack/react-router";
import { AppShell } from "@/components/AppShell";
import { AdminShell } from "@/components/admin/AdminShell";
import { AdminIdMapsPage } from "@/pages/admin/IdMaps";
import { AdminMetricsPage } from "@/pages/admin/Metrics";
import { AdminOverviewPage } from "@/pages/admin/Overview";
import { AdminProviderDetailPage } from "@/pages/admin/ProviderDetail";
import { AdminProvidersListPage } from "@/pages/admin/ProvidersList";
import { AdminSourceDetailPage } from "@/pages/admin/SourceDetail";
import { AdminSourcesListPage } from "@/pages/admin/SourcesList";
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
    if (typeof raw.genre === "string" && raw.genre) search.genre = raw.genre;
    if (typeof raw.tag === "string" && raw.tag) search.tag = raw.tag;
    if (typeof raw.sort === "string" && raw.sort) search.sort = raw.sort;
    if (typeof raw.order === "string" && raw.order) search.order = raw.order;
    if (raw.owned === true || raw.owned === "true") search.owned = true;
    else if (raw.owned === false || raw.owned === "false") search.owned = false;
    const page = Number(raw.page);
    if (Number.isFinite(page) && page > 0) search.page = Math.floor(page);
    if (typeof raw.q === "string" && raw.q.trim()) search.q = raw.q;
    if (raw.view === "list") search.view = "list";
    else if (raw.view === "card") search.view = "card";
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

// `/admin` is a layout route: the AdminShell hosts the auth gate, the
// nav rail, and an <Outlet /> that renders one of the child pages.
// Children declared as relative paths under this layout, so URLs are
// `/admin`, `/admin/sources`, `/admin/sources/$name`, etc.
export const adminLayoutRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin",
  component: AdminShell,
});

export const adminOverviewRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "/",
  component: AdminOverviewPage,
});

export const adminSourcesListRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "sources",
  component: AdminSourcesListPage,
});

export const adminSourceDetailRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "sources/$name",
  component: AdminSourceDetailPage,
});

export const adminProvidersListRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "providers",
  component: AdminProvidersListPage,
});

export const adminProviderDetailRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "providers/$id",
  component: AdminProviderDetailPage,
});

export const adminMetricsRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "metrics",
  component: AdminMetricsPage,
});

export const adminIdMapsRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "id-maps",
  component: AdminIdMapsPage,
});

const routeTree = rootRoute.addChildren([
  feedRoute,
  seriesDetailRoute,
  reviewRoute,
  adminLayoutRoute.addChildren([
    adminOverviewRoute,
    adminSourcesListRoute,
    adminSourceDetailRoute,
    adminProvidersListRoute,
    adminProviderDetailRoute,
    adminMetricsRoute,
    adminIdMapsRoute,
  ]),
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
