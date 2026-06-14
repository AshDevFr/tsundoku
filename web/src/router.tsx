import {
  createRootRoute,
  createRoute,
  createRouter,
  Navigate,
  Outlet,
} from "@tanstack/react-router";
import { AppShell } from "@/components/AppShell";
import { AdminShell } from "@/components/admin/AdminShell";
import { CODEX_STATUS_VALUES } from "@/constants/series";
import { AdminCodexPage } from "@/pages/admin/Codex";
import { AdminDownloadPage } from "@/pages/admin/Download";
import { AdminExportPage } from "@/pages/admin/Export";
import { AdminIdMapsPage } from "@/pages/admin/IdMaps";
import { AdminMaintenancePage } from "@/pages/admin/Maintenance";
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
import { WishlistPage } from "@/pages/WishlistPage";
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

/// Parse the feed filter state out of raw URL search params. Shared by the
/// feed route and the series-detail route so the active filters can ride
/// along into the detail view and back, letting "Back to feed" restore the
/// exact filtered/paginated list the user came from.
function validateFilterSearch(raw: Record<string, unknown>): FilterSearch {
  const search: FilterSearch = {};
  // Kind / status are open-vocab multi-selects: accept a repeated param or a
  // CSV string and keep the values verbatim (the backend doesn't restrict the
  // vocabulary, so don't gate on a known-value list the way codexStatus does).
  const kind = parseStringList(raw.kind);
  if (kind.length > 0) search.kind = kind;
  const status = parseStringList(raw.status);
  if (status.length > 0) search.status = status;
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
  if (raw.wishlisted === true || raw.wishlisted === "true")
    search.wishlisted = true;
  else if (raw.wishlisted === false || raw.wishlisted === "false")
    search.wishlisted = false;
  if (raw.hasReleases === true || raw.hasReleases === "true")
    search.hasReleases = true;
  else if (raw.hasReleases === false || raw.hasReleases === "false")
    search.hasReleases = false;
  if (raw.metadataSource === "manual" || raw.metadataSource === "auto")
    search.metadataSource = raw.metadataSource;
  const page = Number(raw.page);
  if (Number.isFinite(page) && page > 0) search.page = Math.floor(page);
  if (typeof raw.q === "string" && raw.q.trim()) search.q = raw.q;
  const codexStatus = parseStringList(raw.codexStatus).filter((s) =>
    CODEX_STATUS_VALUES.includes(s),
  );
  if (codexStatus.length > 0) search.codexStatus = codexStatus;
  return search;
}

export const feedRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: FeedPage,
  validateSearch: validateFilterSearch,
});

export const seriesDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/series/$id",
  component: SeriesDetailPage,
  // Carry the feed's filter state through the detail view so "Back to feed"
  // can return to the same filtered, paginated list.
  validateSearch: validateFilterSearch,
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

export const adminWishlistRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "wishlist",
  component: WishlistPage,
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

export const adminDownloadRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "download",
  component: AdminDownloadPage,
});

export const adminCodexRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "codex",
  component: AdminCodexPage,
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

export const adminMaintenanceRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "maintenance",
  component: AdminMaintenancePage,
});

export const adminExportRoute = createRoute({
  getParentRoute: () => adminLayoutRoute,
  path: "export",
  component: AdminExportPage,
});

const routeTree = rootRoute.addChildren([
  feedRoute,
  seriesDetailRoute,
  adminLayoutRoute.addChildren([
    adminOverviewRoute,
    adminReviewRoute,
    adminKeptRoute,
    adminWishlistRoute,
    adminSourcesListRoute,
    adminSourceDetailRoute,
    adminProvidersListRoute,
    adminProviderDetailRoute,
    adminDownloadRoute,
    adminCodexRoute,
    adminMetricsRoute,
    adminIdMapsRoute,
    adminMaintenanceRoute,
    adminExportRoute,
  ]),
]);

export const router = createRouter({
  routeTree,
  // Unknown URLs bounce back to the feed rather than dead-ending on a
  // bare "not found" screen.
  defaultNotFoundComponent: () => <Navigate to="/" replace />,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
