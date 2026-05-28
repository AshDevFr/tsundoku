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
import { KeptPage } from "@/pages/KeptPage";
import { ReviewPage } from "@/pages/ReviewPage";
import { SeriesDetailPage } from "@/pages/SeriesDetailPage";
import type { FilterSearch } from "@/stores/filters";

/// Accept either an array (from a repeated query param) or a CSV string
/// (from a pasted link or older URL) and normalize to a deduped list of
/// non-empty entries. Used by both `genres` and `tags`.
function parseStringList(raw: unknown): string[] {
  const out: string[] = [];
  const push = (s: string) => {
    const t = s.trim();
    if (t && !out.includes(t)) out.push(t);
  };
  if (Array.isArray(raw)) {
    for (const v of raw) if (typeof v === "string") push(v);
  } else if (typeof raw === "string") {
    for (const part of raw.split(",")) push(part);
  }
  return out;
}

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
    const genres = parseStringList(raw.genres);
    if (genres.length > 0) search.genres = genres;
    if (raw.genresMode === "all" || raw.genresMode === "any")
      search.genresMode = raw.genresMode;
    const tags = parseStringList(raw.tags);
    if (tags.length > 0) search.tags = tags;
    if (raw.tagsMode === "all" || raw.tagsMode === "any")
      search.tagsMode = raw.tagsMode;
    if (typeof raw.sort === "string" && raw.sort) search.sort = raw.sort;
    if (typeof raw.order === "string" && raw.order) search.order = raw.order;
    if (raw.owned === true || raw.owned === "true") search.owned = true;
    else if (raw.owned === false || raw.owned === "false") search.owned = false;
    if (raw.hasReleases === true || raw.hasReleases === "true")
      search.hasReleases = true;
    else if (raw.hasReleases === false || raw.hasReleases === "false")
      search.hasReleases = false;
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

export const adminReviewRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "review",
  component: ReviewPage,
});

export const adminKeptRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "kept",
  component: KeptPage,
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
  adminLayoutRoute.addChildren([
    adminOverviewRoute,
    adminReviewRoute,
    adminKeptRoute,
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
