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
import { ADMIN_TEST_TOKEN, resetSeries } from "@/mocks/handlers";
import { useAdminAuth } from "@/stores/auth";
import { WishlistPage } from "./WishlistPage";

function makeRouter() {
  const root = createRootRoute({ component: Outlet });
  const admin = createRoute({
    getParentRoute: () => root,
    path: "/admin",
    component: AdminShell,
  });
  const wishlist = createRoute({
    getParentRoute: () => admin,
    path: "wishlist",
    component: WishlistPage,
  });
  // A stub series-detail route so the SeriesCard <Link to="/series/$id">
  // resolves in the test router.
  const series = createRoute({
    getParentRoute: () => root,
    path: "/series/$id",
    component: () => null,
  });
  return createRouter({
    routeTree: root.addChildren([admin.addChildren([wishlist]), series]),
    history: createMemoryHistory({ initialEntries: ["/admin/wishlist"] }),
  });
}

function renderWishlist() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const router = makeRouter();
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

describe("WishlistPage", () => {
  beforeEach(() => {
    resetSeries();
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
  });

  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("shows the empty state when nothing is wishlisted", async () => {
    renderWishlist();
    expect(
      await screen.findByText(/Nothing on the wishlist yet/, undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
  });

  it("adds a series from MangaBaka and it appears on the wishlist", async () => {
    renderWishlist();
    await screen.findByText(/Nothing on the wishlist yet/, undefined, {
      timeout: 3000,
    });

    // Open the add modal and search by title.
    fireEvent.click(screen.getByTestId("wishlist-add-open"));
    const titleInput = await screen.findByTestId("search-title");
    fireEvent.change(titleInput, { target: { value: "Berserk" } });

    // The debounced search returns canned hits; add the first one.
    const addBtn = await screen.findByTestId("link-hit-mb-1", undefined, {
      timeout: 3000,
    });
    expect(addBtn).toHaveTextContent("Add");
    fireEvent.click(addBtn);

    // The new series is created + wishlisted, the modal closes, and the card
    // shows up once the list refetches.
    await waitFor(
      () =>
        expect(screen.getByText("MangaBaka series mb-1")).toBeInTheDocument(),
      { timeout: 3000 },
    );
    expect(screen.getByText(/Added "Berserk" to the wishlist/)).toBeTruthy();
  });
});
