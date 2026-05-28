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
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { AdminShell } from "@/components/admin/AdminShell";
import { ADMIN_TEST_TOKEN, resetReviewQueue } from "@/mocks/handlers";
import { useAdminAuth } from "@/stores/auth";
import { KeptPage } from "./KeptPage";

function makeRouter() {
  const root = createRootRoute({ component: Outlet });
  const admin = createRoute({
    getParentRoute: () => root,
    path: "/admin",
    component: AdminShell,
  });
  const kept = createRoute({
    getParentRoute: () => admin,
    path: "kept",
    component: KeptPage,
  });
  return createRouter({
    routeTree: root.addChildren([admin.addChildren([kept])]),
    history: createMemoryHistory({ initialEntries: ["/admin/kept"] }),
  });
}

function renderKept() {
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

describe("KeptPage", () => {
  beforeEach(() => {
    resetReviewQueue();
    useAdminAuth.getState().clear();
  });

  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("lists standalone releases with their links once authed", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderKept();
    expect(
      await screen.findByText(/Shonen Jump Guide to Making Manga/, undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    expect(screen.getByText(/1 standalone release/)).toBeInTheDocument();
    const card = screen.getByTestId("kept-card-nyaa:7001");
    const magnet = Array.from(card.querySelectorAll("a")).find(
      (a) => a.textContent?.trim() === "magnet",
    );
    expect(magnet).toBeTruthy();
  });

  it("surfaces extracted links and an expandable description", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderKept();
    await screen.findByText(/Shonen Jump Guide to Making Manga/, undefined, {
      timeout: 3000,
    });
    const card = screen.getByTestId("kept-card-nyaa:7001");
    // The scraped provider link is surfaced, like in the review queue.
    expect(card.querySelector('[data-testid="extracted-links"]')).toBeTruthy();

    // The description sits behind a show/hide toggle.
    const desc = card.querySelector<HTMLElement>(
      '[data-testid="description-block"]',
    );
    if (!desc) throw new Error("description block not rendered");
    const toggle = within(desc).getByRole("button");
    expect(toggle).toHaveTextContent("show");
    fireEvent.click(toggle);
    expect(toggle).toHaveTextContent("hide");
    expect(within(desc).getByText(/official guidebook/i)).toBeInTheDocument();
  });

  it("re-resolves a kept release via the Re-resolve button", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderKept();
    await screen.findByText(/Shonen Jump Guide to Making Manga/, undefined, {
      timeout: 3000,
    });
    const btn = screen.getByTestId("re-resolve-nyaa:7001");
    fireEvent.click(btn);
    // The retry mock leaves the row in place; assert the success toast fires
    // and the card is still present (no crash, request succeeded).
    await waitFor(
      () => {
        expect(screen.getByText(/Re-running resolver/)).toBeInTheDocument();
      },
      { timeout: 3000 },
    );
  });
});
