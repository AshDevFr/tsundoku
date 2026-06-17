import { MantineProvider } from "@mantine/core";
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
import { describe, expect, it } from "vitest";
import { AppShell } from "./AppShell";

// Render the AppShell as the root layout (matching prod, where the root route
// wraps every page in <AppShell>) with a few stub destinations so the drawer's
// Links resolve.
function renderShell(initial = "/") {
  const root = createRootRoute({
    component: () => (
      <AppShell>
        <Outlet />
      </AppShell>
    ),
  });
  const stub = (path: string) =>
    createRoute({
      getParentRoute: () => root,
      path,
      component: () => null,
    });
  const router = createRouter({
    routeTree: root.addChildren([
      stub("/"),
      stub("/admin"),
      stub("/admin/review"),
      stub("/admin/wishlist"),
    ]),
    history: createMemoryHistory({ initialEntries: [initial] }),
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <MantineProvider>
      <QueryClientProvider client={client}>
        {/* biome-ignore lint/suspicious/noExplicitAny: test route tree shape differs from prod */}
        <RouterProvider router={router as any} />
      </QueryClientProvider>
    </MantineProvider>,
  );
}

describe("AppShell mobile navigation", () => {
  it("renders a burger that starts in the closed state", async () => {
    renderShell();
    expect(
      await screen.findByRole("button", { name: /open navigation/i }),
    ).toBeInTheDocument();
  });

  it("exposes the top-level destinations in the drawer", async () => {
    renderShell();
    fireEvent.click(
      await screen.findByRole("button", { name: /open navigation/i }),
    );
    expect(screen.getByTestId("mobile-nav-review")).toBeInTheDocument();
    expect(screen.getByTestId("mobile-nav-wishlist")).toBeInTheDocument();
    expect(screen.getByTestId("mobile-nav-admin")).toBeInTheDocument();
  });

  it("toggles the burger label when opened", async () => {
    renderShell();
    const burger = await screen.findByRole("button", {
      name: /open navigation/i,
    });
    fireEvent.click(burger);
    expect(
      await screen.findByRole("button", { name: /close navigation/i }),
    ).toBeInTheDocument();
  });

  it("closes the drawer after navigating to a destination", async () => {
    renderShell();
    fireEvent.click(
      await screen.findByRole("button", { name: /open navigation/i }),
    );
    // Drawer open: burger now offers to close.
    await screen.findByRole("button", { name: /close navigation/i });
    fireEvent.click(screen.getByTestId("mobile-nav-review"));
    // Navigating collapses the drawer, so the burger reverts to "open".
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /open navigation/i }),
      ).toBeInTheDocument(),
    );
  });
});
