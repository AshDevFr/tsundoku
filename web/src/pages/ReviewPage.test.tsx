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
import { ReviewPage } from "./ReviewPage";

function makeRouter() {
  const root = createRootRoute({ component: Outlet });
  const review = createRoute({
    getParentRoute: () => root,
    path: "/review",
    component: ReviewPage,
  });
  return createRouter({
    routeTree: root.addChildren([review]),
    history: createMemoryHistory({ initialEntries: ["/review"] }),
  });
}

function renderReview() {
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

describe("ReviewPage", () => {
  beforeEach(() => {
    resetReviewQueue();
    useAdminAuth.getState().clear();
  });

  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("requires admin token before showing the queue", async () => {
    renderReview();
    expect(await screen.findByText(/Admin sign-in/i)).toBeInTheDocument();
    expect(screen.queryByText(/Review queue/)).not.toBeInTheDocument();
  });

  it("accepts a token and reveals the unresolved queue", async () => {
    renderReview();
    const input = await screen.findByTestId("admin-token-input");
    fireEvent.change(input, { target: { value: ADMIN_TEST_TOKEN } });
    fireEvent.click(screen.getByRole("button", { name: "Save token" }));
    expect(
      await screen.findByText(/Review queue/, undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
    expect(
      await screen.findByText(/Mystery Series/, undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
    expect(screen.getByText(/Unknown Title v05/)).toBeInTheDocument();
    expect(
      screen.getByText(/2 releases awaiting a decision/),
    ).toBeInTheDocument();
  });

  it("links a release by picking a candidate and drops it from the queue", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series/, undefined, { timeout: 3000 });
    // Two candidates render for the first card: Chainsaw Man (1), Solo Leveling (3).
    fireEvent.click(screen.getByTestId("link-candidate-1"));
    await waitFor(
      () => {
        expect(
          screen.queryByTestId("review-card-nyaa:9001"),
        ).not.toBeInTheDocument();
      },
      { timeout: 3000 },
    );
    expect(
      screen.getByText(/1 release awaiting a decision/),
    ).toBeInTheDocument();
  });

  it("rejects a release via the reject button", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series/, undefined, { timeout: 3000 });
    const card = screen.getByTestId("review-card-nyaa:9001");
    const rejectBtn = Array.from(card.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === "Reject",
    );
    if (!rejectBtn) throw new Error("reject button not rendered");
    fireEvent.click(rejectBtn);
    await waitFor(
      () => {
        expect(
          screen.queryByTestId("review-card-nyaa:9001"),
        ).not.toBeInTheDocument();
      },
      { timeout: 3000 },
    );
  });

  it("opens the manual link modal and submits provider+externalId", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Unknown Title v05/, undefined, { timeout: 3000 });
    const card = screen.getByTestId("review-card-nyaa:9002");
    const manualBtn = Array.from(card.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Link by external ID"),
    );
    if (!manualBtn) throw new Error("manual-link button not rendered");
    fireEvent.click(manualBtn);
    const dialog = await screen.findByRole("dialog");
    const idInput = await waitFor(() => {
      const el = dialog.querySelector<HTMLInputElement>(
        '[data-testid="manual-external-id"]',
      );
      if (!el) throw new Error("manual-external-id input not rendered");
      return el;
    });
    fireEvent.change(idInput, { target: { value: "1234" } });
    const submit = Array.from(dialog.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === "Link release",
    );
    if (!submit) throw new Error("Link release button not rendered");
    fireEvent.click(submit);
    await waitFor(
      () => {
        expect(
          screen.queryByTestId("review-card-nyaa:9002"),
        ).not.toBeInTheDocument();
      },
      { timeout: 3000 },
    );
  });
});
