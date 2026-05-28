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
import { ReviewPage } from "./ReviewPage";

function makeRouter() {
  const root = createRootRoute({ component: Outlet });
  const admin = createRoute({
    getParentRoute: () => root,
    path: "/admin",
    component: AdminShell,
  });
  const review = createRoute({
    getParentRoute: () => admin,
    path: "review",
    component: ReviewPage,
  });
  return createRouter({
    routeTree: root.addChildren([admin.addChildren([review])]),
    history: createMemoryHistory({ initialEntries: ["/admin/review"] }),
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
      await screen.findByText(/Mystery Series v01/, undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    expect(screen.getByText(/Unknown Title v05/)).toBeInTheDocument();
    expect(
      screen.getByText(/2 releases awaiting a decision/),
    ).toBeInTheDocument();
  });

  it("shows the torrent file list behind a toggle and candidate vol/chapter counts", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });
    const card = screen.getByTestId("review-card-nyaa:9001");

    // The file list shows the file count and toggles open/closed.
    const filesBlock = card.querySelector(
      '[data-testid="files-block"]',
    ) as HTMLElement;
    expect(filesBlock).toBeInTheDocument();
    expect(filesBlock.textContent).toMatch(/files \(3\)/i);
    const toggle = within(filesBlock).getByRole("button");
    expect(toggle).toHaveTextContent("show");
    fireEvent.click(toggle);
    expect(toggle).toHaveTextContent("hide");
    expect(
      within(filesBlock).getByText("Mystery_Series_v02.cbz"),
    ).toBeInTheDocument();

    // Candidate counts: Chainsaw Man has both vols + chapters; Solo Leveling
    // only chapters (null volume is omitted).
    expect(within(card).getByText("11 vols · 97 ch")).toBeInTheDocument();
    expect(within(card).getByText("179 ch")).toBeInTheDocument();
  });

  it("links a release by picking a candidate and drops it from the queue", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });
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
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });
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

  it("keeps a release via the keep button and drops it from the queue", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });
    const card = screen.getByTestId("review-card-nyaa:9001");
    const keepBtn = Array.from(card.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === "Keep",
    );
    if (!keepBtn) throw new Error("keep button not rendered");
    fireEvent.click(keepBtn);
    await waitFor(
      () => {
        expect(
          screen.queryByTestId("review-card-nyaa:9001"),
        ).not.toBeInTheDocument();
      },
      { timeout: 3000 },
    );
  });

  it("opens the search modal, pastes an external ID, and links the release", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Unknown Title v05/, undefined, { timeout: 3000 });
    const card = screen.getByTestId("review-card-nyaa:9002");
    const searchBtn = Array.from(card.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Search provider"),
    );
    if (!searchBtn) throw new Error("Search provider button not rendered");
    fireEvent.click(searchBtn);
    const dialog = await screen.findByRole("dialog");
    const idInput = await waitFor(() => {
      const el = dialog.querySelector<HTMLInputElement>(
        '[data-testid="search-external-id"]',
      );
      if (!el) throw new Error("search-external-id input not rendered");
      return el;
    });
    fireEvent.change(idInput, { target: { value: "1234" } });
    // The externalId path returns one hit; click its Link button.
    const linkBtn = await screen.findByTestId("link-hit-1234", undefined, {
      timeout: 3000,
    });
    fireEvent.click(linkBtn);
    await waitFor(
      () => {
        expect(
          screen.queryByTestId("review-card-nyaa:9002"),
        ).not.toBeInTheDocument();
      },
      { timeout: 3000 },
    );
  });

  it("creates a manual series and links the release in one step", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Unknown Title v05/, undefined, { timeout: 3000 });
    const card = screen.getByTestId("review-card-nyaa:9002");
    const createBtn = Array.from(card.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === "Create series",
    );
    if (!createBtn) throw new Error("Create series button not rendered");
    fireEvent.click(createBtn);

    const dialog = await screen.findByRole("dialog");
    const titleInput = await waitFor(() => {
      const el = dialog.querySelector<HTMLInputElement>(
        '[data-testid="create-series-title"]',
      );
      if (!el) throw new Error("create-series-title input not rendered");
      return el;
    });
    fireEvent.change(titleInput, { target: { value: "Hand Made Series" } });
    fireEvent.click(screen.getByTestId("create-series-submit"));

    await waitFor(
      () => {
        expect(
          screen.queryByTestId("review-card-nyaa:9002"),
        ).not.toBeInTheDocument();
      },
      { timeout: 3000 },
    );
  });

  it("links a release to an existing catalog series via the Link existing modal", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });
    const card = screen.getByTestId("review-card-nyaa:9001");
    const linkExistingBtn = Array.from(card.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === "Link existing",
    );
    if (!linkExistingBtn) throw new Error("Link existing button not rendered");
    fireEvent.click(linkExistingBtn);

    const dialog = await screen.findByRole("dialog");
    const search = await waitFor(() => {
      const el = dialog.querySelector<HTMLInputElement>(
        '[data-testid="link-existing-search"]',
      );
      if (!el) throw new Error("link-existing-search input not rendered");
      return el;
    });
    // Search the local catalog for an existing series.
    fireEvent.change(search, { target: { value: "Chainsaw" } });
    const linkBtn = await screen.findByTestId("link-existing-1", undefined, {
      timeout: 3000,
    });
    fireEvent.click(linkBtn);

    await waitFor(
      () => {
        expect(
          screen.queryByTestId("review-card-nyaa:9001"),
        ).not.toBeInTheDocument();
      },
      { timeout: 3000 },
    );
  });

  it("renders the cleanup trail (cleaned query + rule chips) on each card", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });
    const card = screen.getByTestId("review-card-nyaa:9001");
    const trail = card.querySelector('[data-testid="cleanup-trail"]');
    if (!trail) throw new Error("cleanup-trail not rendered");
    // Primary search query is shown as monospaced text.
    expect(trail.textContent).toContain("Mystery Series");
    // Rule chips render the rule names.
    expect(trail.textContent).toContain("strip_brackets");
    expect(trail.textContent).toContain("strip_parens");
  });

  it("filters the queue by title search", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });
    // Both cards present before filtering.
    expect(screen.getByTestId("review-card-nyaa:9002")).toBeInTheDocument();

    fireEvent.change(screen.getByTestId("review-search"), {
      target: { value: "Unknown" },
    });

    // The non-matching card drops out (debounced + refetched).
    await waitFor(
      () => {
        expect(
          screen.queryByTestId("review-card-nyaa:9001"),
        ).not.toBeInTheDocument();
      },
      { timeout: 3000 },
    );
    expect(screen.getByTestId("review-card-nyaa:9002")).toBeInTheDocument();
  });

  it("clears filters to restore the full queue", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });
    fireEvent.change(screen.getByTestId("review-search"), {
      target: { value: "Unknown" },
    });
    await waitFor(
      () => {
        expect(
          screen.queryByTestId("review-card-nyaa:9001"),
        ).not.toBeInTheDocument();
      },
      { timeout: 3000 },
    );

    fireEvent.click(screen.getByTestId("review-clear-filters"));
    await waitFor(
      () => {
        expect(screen.getByTestId("review-card-nyaa:9001")).toBeInTheDocument();
      },
      { timeout: 3000 },
    );
  });

  it("bulk-rejects the selected releases after confirmation", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });

    // Select every release on the page; the bulk toolbar appears.
    fireEvent.click(screen.getByTestId("select-all-page"));
    expect(screen.getByTestId("bulk-action-bar")).toHaveTextContent(
      "2 selected",
    );

    // Reject opens a confirmation modal, not an immediate action.
    fireEvent.click(screen.getByTestId("bulk-reject"));
    await screen.findByRole("dialog");
    fireEvent.click(screen.getByTestId("confirm-bulk-reject"));

    await waitFor(
      () => {
        expect(
          screen.queryByTestId("review-card-nyaa:9001"),
        ).not.toBeInTheDocument();
        expect(
          screen.queryByTestId("review-card-nyaa:9002"),
        ).not.toBeInTheDocument();
      },
      { timeout: 3000 },
    );
  });

  it("cancels a bulk reject and leaves the releases in the queue", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });

    fireEvent.click(screen.getByTestId("select-all-page"));
    fireEvent.click(screen.getByTestId("bulk-reject"));
    await screen.findByRole("dialog");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    // Both cards survive the cancelled action.
    expect(screen.getByTestId("review-card-nyaa:9001")).toBeInTheDocument();
    expect(screen.getByTestId("review-card-nyaa:9002")).toBeInTheDocument();
  });

  it("bulk-retries the selected releases and clears the selection", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });

    fireEvent.click(screen.getByTestId("select-nyaa:9001"));
    expect(screen.getByTestId("bulk-action-bar")).toHaveTextContent(
      "1 selected",
    );
    fireEvent.click(screen.getByTestId("bulk-retry"));

    // Retry is a background batch: the rows stay, but the selection clears
    // (the toolbar disappears) once the request resolves.
    await waitFor(
      () => {
        expect(screen.queryByTestId("bulk-action-bar")).not.toBeInTheDocument();
      },
      { timeout: 3000 },
    );
    expect(screen.getByTestId("review-card-nyaa:9001")).toBeInTheDocument();
  });

  it("shows every search query when the cleaner produced more than one", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Unknown Title v05/, undefined, { timeout: 3000 });
    const card = screen.getByTestId("review-card-nyaa:9002");
    const trail = card.querySelector('[data-testid="cleanup-trail"]');
    if (!trail) throw new Error("cleanup-trail not rendered");
    // Both the full title and the discounted subtitle head are shown.
    expect(trail.textContent).toContain("Unknown Title - A Story");
    expect(trail.textContent).toContain("Unknown Title");
    expect(trail.textContent).toContain("split_subtitle");
  });
});
