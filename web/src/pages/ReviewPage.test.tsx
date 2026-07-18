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
import {
  ADMIN_TEST_TOKEN,
  makeUnresolved,
  resetReviewQueue,
  seedReviewQueue,
} from "@/mocks/handlers";
import { useAdminAuth } from "@/stores/auth";
import { DEFAULT_REVIEW_PAGE_SIZE, useUiPrefs } from "@/stores/uiPrefs";
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
    // Page size is persisted in localStorage; reset it so a size change in one
    // test doesn't bleed into the next.
    useUiPrefs.getState().setReviewPageSize(DEFAULT_REVIEW_PAGE_SIZE);
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

    // Each candidate surfaces its series format (manga / manhwa / …).
    expect(within(card).getByText("manga")).toBeInTheDocument();
    expect(within(card).getByText("manhwa")).toBeInTheDocument();
  });

  it("surfaces the post's Information link even when it is not a matching provider link", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });
    const card = screen.getByTestId("review-card-nyaa:9001");

    const infoBlock = card.querySelector(
      '[data-testid="information-link"]',
    ) as HTMLElement;
    expect(infoBlock).toBeInTheDocument();
    // The hostname is the visible label; the full URL is the href.
    const anchor = within(infoBlock).getByRole("link");
    expect(anchor).toHaveTextContent("sevenseasentertainment.com");
    expect(anchor).toHaveAttribute(
      "href",
      "https://sevenseasentertainment.com/series/mystery-series/",
    );
  });

  it("offers comment-suggested links as a one-click seeded lookup", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });
    const card = screen.getByTestId("review-card-nyaa:9001");
    const block = within(card).getByTestId("comment-suggestions");
    expect(block.textContent).toMatch(/Suggested in comments/i);
    const btn = within(card).getByTestId("comment-suggestion-mangaupdates");
    fireEvent.click(btn);
    // The modal opens with the External ID pre-filled from the comment link.
    const input = await screen.findByTestId("search-external-id");
    expect(input).toHaveValue(
      "https://www.mangaupdates.com/series/ylx5wzn/mystery-series",
    );
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

  it("shift+clicks to select a contiguous range of releases", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    // Five rows so a mid-list range has a meaningful interior. Seed before
    // render so the first query already sees them.
    seedReviewQueue(
      Array.from({ length: 5 }, (_, i) =>
        makeUnresolved(`nyaa:81${i}`, {
          externalId: `81${i}`,
          title: `Range Series ${i} v01`,
          resolutionStatus: "unresolved",
        }),
      ),
    );
    renderReview();
    await screen.findByTestId("review-card-nyaa:810");

    // Anchor on the second row, then shift+click the fourth: rows 1..3 select.
    fireEvent.click(screen.getByTestId("select-nyaa:811"));
    fireEvent.click(screen.getByTestId("select-nyaa:813"), { shiftKey: true });

    expect(screen.getByTestId("bulk-action-bar")).toHaveTextContent(
      "3 selected",
    );
    for (const id of ["nyaa:811", "nyaa:812", "nyaa:813"]) {
      expect(screen.getByTestId(`select-${id}`)).toBeChecked();
    }
    // The rows outside the range stay untouched.
    expect(screen.getByTestId("select-nyaa:810")).not.toBeChecked();
    expect(screen.getByTestId("select-nyaa:814")).not.toBeChecked();
  });

  it("anchors range selection at the last single click (not the page top)", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    seedReviewQueue(
      Array.from({ length: 4 }, (_, i) =>
        makeUnresolved(`nyaa:82${i}`, {
          externalId: `82${i}`,
          title: `Anchor Series ${i} v01`,
          resolutionStatus: "unresolved",
        }),
      ),
    );
    renderReview();
    await screen.findByTestId("review-card-nyaa:820");

    // A plain click on the third row re-anchors there; shift+clicking the
    // first row extends back up over rows 0..2, leaving the last row alone.
    fireEvent.click(screen.getByTestId("select-nyaa:822"));
    fireEvent.click(screen.getByTestId("select-nyaa:820"), { shiftKey: true });

    expect(screen.getByTestId("bulk-action-bar")).toHaveTextContent(
      "3 selected",
    );
    for (const id of ["nyaa:820", "nyaa:821", "nyaa:822"]) {
      expect(screen.getByTestId(`select-${id}`)).toBeChecked();
    }
    expect(screen.getByTestId("select-nyaa:823")).not.toBeChecked();
  });

  it("collapses a single card to its header via the chevron and expands it back", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });
    const card = screen.getByTestId("review-card-nyaa:9001");
    // Expanded by default: the candidate list is in the DOM.
    expect(within(card).getByTestId("candidate-1")).toBeInTheDocument();

    fireEvent.click(within(card).getByTestId("collapse-nyaa:9001"));
    // Collapsed: the detail body (candidates) unmounts, but the title stays.
    await waitFor(() => {
      expect(within(card).queryByTestId("candidate-1")).not.toBeInTheDocument();
    });
    expect(within(card).getByText(/Mystery Series v01/)).toBeInTheDocument();

    // Toggling again brings the body back.
    fireEvent.click(within(card).getByTestId("collapse-nyaa:9001"));
    expect(within(card).getByTestId("candidate-1")).toBeInTheDocument();
  });

  it("collapses and expands every card via the toolbar toggle", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });
    const toggle = screen.getByTestId("toggle-collapse-all");
    expect(toggle).toHaveTextContent("Collapse all");

    fireEvent.click(toggle);
    await waitFor(() => {
      expect(screen.queryByTestId("candidate-1")).not.toBeInTheDocument();
    });
    // Both cards' headers survive; the toggle flips to "Expand all".
    expect(screen.getByTestId("review-card-nyaa:9001")).toBeInTheDocument();
    expect(screen.getByTestId("review-card-nyaa:9002")).toBeInTheDocument();
    expect(toggle).toHaveTextContent("Expand all");

    fireEvent.click(toggle);
    await waitFor(() => {
      expect(screen.getByTestId("candidate-1")).toBeInTheDocument();
    });
  });

  it("sorts the queue by title via the sort control", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });

    // The mock orders by title when sort=title_asc; assert the cards reorder
    // in the DOM ("Mystery…" before "Unknown…").
    const select = screen.getByTestId("review-sort");
    fireEvent.click(select);
    fireEvent.click(await screen.findByText("Title A→Z"));

    await waitFor(() => {
      const cards = screen.getAllByTestId(/^review-card-/);
      expect(cards[0]).toHaveAttribute("data-testid", "review-card-nyaa:9001");
      expect(cards[1]).toHaveAttribute("data-testid", "review-card-nyaa:9002");
    });

    // Reverse: "Unknown…" should now come first.
    fireEvent.click(select);
    fireEvent.click(await screen.findByText("Title Z→A"));
    await waitFor(() => {
      const cards = screen.getAllByTestId(/^review-card-/);
      expect(cards[0]).toHaveAttribute("data-testid", "review-card-nyaa:9002");
    });
  });

  it("changes the page size via the per-page selector", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    // 25 rows: one more than the default page of 20, so a second page exists.
    seedReviewQueue(
      Array.from({ length: 25 }, (_, i) =>
        makeUnresolved(`nyaa:83${String(i).padStart(2, "0")}`, {
          externalId: `83${String(i).padStart(2, "0")}`,
          title: `Paged Series ${i} v01`,
          resolutionStatus: "unresolved",
        }),
      ),
    );
    renderReview();
    await screen.findByTestId("review-card-nyaa:8300");

    // The default page shows 20 of the 25 rows; pagination is present.
    expect(screen.getAllByTestId(/^review-card-/)).toHaveLength(20);
    expect(
      screen.queryByTestId("review-card-nyaa:8324"),
    ).not.toBeInTheDocument();

    // Bump to 50/page: the whole queue now fits on one page.
    const select = screen.getByTestId("review-page-size");
    fireEvent.click(select);
    fireEvent.click(await screen.findByText("50 / page"));

    await waitFor(() => {
      expect(screen.getAllByTestId(/^review-card-/)).toHaveLength(25);
    });
    expect(screen.getByTestId("review-card-nyaa:8324")).toBeInTheDocument();
  });

  it("bulk-links the selected releases to a catalog series", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });

    fireEvent.click(screen.getByTestId("select-all-page"));
    expect(screen.getByTestId("bulk-action-bar")).toHaveTextContent(
      "2 selected",
    );

    fireEvent.click(screen.getByTestId("bulk-link"));
    const dialog = await screen.findByRole("dialog");
    // Catalog is the default mode; search and pick an existing series.
    const search = await waitFor(() => {
      const el = dialog.querySelector<HTMLInputElement>(
        '[data-testid="link-existing-search"]',
      );
      if (!el) throw new Error("link-existing-search input not rendered");
      return el;
    });
    fireEvent.change(search, { target: { value: "Chainsaw" } });
    const linkBtn = await screen.findByTestId("link-existing-1", undefined, {
      timeout: 3000,
    });
    fireEvent.click(linkBtn);

    // Both selected releases leave the queue.
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

  it("bulk-creates a manual series and links the whole selection", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });

    fireEvent.click(screen.getByTestId("select-all-page"));
    fireEvent.click(screen.getByTestId("bulk-create-series"));

    const dialog = await screen.findByRole("dialog");
    const titleInput = await waitFor(() => {
      const el = dialog.querySelector<HTMLInputElement>(
        '[data-testid="bulk-create-series-title"]',
      );
      if (!el) throw new Error("bulk-create-series-title input not rendered");
      return el;
    });
    fireEvent.change(titleInput, { target: { value: "Bundled Series" } });
    fireEvent.click(screen.getByTestId("bulk-create-series-submit"));

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

  it("hides the bulk link/create buttons' effect under select-all-matching", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderReview();
    await screen.findByText(/Mystery Series v01/, undefined, { timeout: 3000 });

    fireEvent.click(screen.getByTestId("select-all-page"));
    // With only 2 of 2 matching, there's no "select all matching" link, so
    // the link/create buttons stay enabled for the explicit selection.
    expect(screen.getByTestId("bulk-link")).not.toBeDisabled();
    expect(screen.getByTestId("bulk-create-series")).not.toBeDisabled();
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

  // --- Release grouping panel ---------------------------------------------

  /// Open the collapsible group panel.
  function openGroupPanel() {
    fireEvent.click(screen.getByTestId("release-group-toggle"));
  }

  /// Click the group row whose label contains `text`.
  async function clickGroupChip(text: string) {
    const row = (await screen.findAllByTestId("release-group-chip")).find(
      (el) => el.textContent?.includes(text),
    );
    if (!row) throw new Error(`no group chip matching ${text}`);
    fireEvent.click(row);
  }

  it("clusters the queue and scopes the list to a clicked group", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    seedReviewQueue([
      ...Array.from({ length: 3 }, (_, i) =>
        makeUnresolved(`grp:op${i}`, {
          externalId: `op${i}`,
          title: `One Piece v0${i + 1}`,
          searchQueries: ["one piece"],
          resolutionStatus: "unresolved",
        }),
      ),
      makeUnresolved("grp:bl0", {
        externalId: "bl0",
        title: "Bleach v01",
        searchQueries: ["bleach"],
        resolutionStatus: "unresolved",
      }),
    ]);
    renderReview();
    await screen.findByTestId("review-card-grp:op0");

    openGroupPanel();
    // Only the 3-member cluster qualifies (>1); the lone Bleach release doesn't.
    await screen.findByTestId("release-group-chip");
    const panel = screen.getByTestId("release-group-panel");
    expect(panel.textContent).toContain("one piece");
    expect(panel.textContent).toContain("×3");

    await clickGroupChip("one piece");

    // The list narrows to the group; the unrelated Bleach card drops out.
    await waitFor(() => {
      expect(
        screen.queryByTestId("review-card-grp:bl0"),
      ).not.toBeInTheDocument();
    });
    expect(screen.getByTestId("review-card-grp:op0")).toBeInTheDocument();
    // The active scope is surfaced as a removable badge.
    expect(screen.getByTestId("review-active-group")).toHaveTextContent(
      "Group: one piece",
    );
    // Picking a group collapses its members by default (toolbar flips to
    // "Expand all" once every loaded card is collapsed).
    await waitFor(() => {
      expect(screen.getByTestId("toggle-collapse-all")).toHaveTextContent(
        "Expand all",
      );
    });
  });

  it("widens the clusters when breadth is loosened", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    // "one piece" is primary on one release and only a secondary variant on the
    // other, so the cluster exists at breadth ≥ 2 but not at the tight default.
    seedReviewQueue([
      makeUnresolved("brd:a", {
        externalId: "a",
        title: "Primary v01",
        searchQueries: ["one piece"],
        resolutionStatus: "unresolved",
      }),
      makeUnresolved("brd:b", {
        externalId: "b",
        title: "Secondary v01",
        searchQueries: ["bleach", "one piece"],
        resolutionStatus: "unresolved",
      }),
    ]);
    renderReview();
    await screen.findByTestId("review-card-brd:a");

    openGroupPanel();
    // Tight (breadth 1): no cluster — each primary query is unique.
    expect(
      await screen.findByTestId("release-group-empty"),
    ).toBeInTheDocument();

    // Loosen to Medium (breadth 2): the secondary "one piece" now joins.
    const panel = screen.getByTestId("release-group-panel");
    fireEvent.click(within(panel).getByText("Medium"));
    await screen.findByTestId("release-group-chip");
    expect(panel.textContent).toContain("one piece");
    expect(panel.textContent).toContain("×2");
  });

  it("clears the group scope to restore the full queue", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    seedReviewQueue([
      makeUnresolved("clr:op0", {
        externalId: "op0",
        title: "One Piece v01",
        searchQueries: ["one piece"],
        resolutionStatus: "unresolved",
      }),
      makeUnresolved("clr:op1", {
        externalId: "op1",
        title: "One Piece v02",
        searchQueries: ["one piece"],
        resolutionStatus: "unresolved",
      }),
      makeUnresolved("clr:bl0", {
        externalId: "bl0",
        title: "Bleach v01",
        searchQueries: ["bleach"],
        resolutionStatus: "unresolved",
      }),
    ]);
    renderReview();
    await screen.findByTestId("review-card-clr:bl0");

    openGroupPanel();
    await clickGroupChip("one piece");
    await waitFor(() => {
      expect(
        screen.queryByTestId("review-card-clr:bl0"),
      ).not.toBeInTheDocument();
    });

    // Removing the active-group badge restores the unscoped queue.
    fireEvent.click(
      within(screen.getByTestId("review-active-group")).getByRole("button"),
    );
    await waitFor(() => {
      expect(screen.getByTestId("review-card-clr:bl0")).toBeInTheDocument();
    });
  });

  // Renders 22 review cards and walks the full escalate → confirm → drain
  // flow; on slow CI runners it legitimately takes ~5s, right at vitest's
  // default 5000ms timeout, so it gets an explicit one.
  it("bulk-rejects an entire group via select-all-matching", {
    timeout: 15_000,
  }, async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    // 21 group members (one more than a page) so "select all matching" surfaces
    // and the bulk request carries the searchQuery filter, not explicit ids.
    seedReviewQueue([
      ...Array.from({ length: 21 }, (_, i) =>
        makeUnresolved(`bgr:op${i}`, {
          externalId: `op${i}`,
          title: `One Piece v${String(i + 1).padStart(2, "0")}`,
          searchQueries: ["one piece"],
          resolutionStatus: "unresolved",
        }),
      ),
      makeUnresolved("bgr:bl0", {
        externalId: "bl0",
        title: "Bleach v01",
        searchQueries: ["bleach"],
        resolutionStatus: "unresolved",
      }),
    ]);
    renderReview();
    await screen.findByTestId("review-card-bgr:op0");

    openGroupPanel();
    await clickGroupChip("one piece");
    // Wait for the list to narrow to the group (Bleach drops out).
    await waitFor(() => {
      expect(
        screen.queryByTestId("review-card-bgr:bl0"),
      ).not.toBeInTheDocument();
    });

    // Select the page, which reveals the "select all matching" escalation
    // (21 in the group > 20 on the page), then act on the whole group.
    fireEvent.click(screen.getByTestId("select-all-page"));
    fireEvent.click(await screen.findByTestId("select-all-matching"));
    expect(screen.getByTestId("bulk-action-bar")).toHaveTextContent(
      "21 selected",
    );

    fireEvent.click(screen.getByTestId("bulk-reject"));
    await screen.findByRole("dialog");
    fireEvent.click(screen.getByTestId("confirm-bulk-reject"));

    // The whole group drains; the out-of-group Bleach release survives, proving
    // the bulk action was scoped by searchQuery and not a blanket reject.
    await waitFor(
      () => {
        expect(
          screen.queryByTestId("review-card-bgr:op0"),
        ).not.toBeInTheDocument();
      },
      { timeout: 3000 },
    );
    // Clear via the always-present active-group badge (the in-panel "Clear
    // group" link is gone once the drained group has no chips).
    fireEvent.click(
      within(screen.getByTestId("review-active-group")).getByRole("button"),
    );
    await waitFor(() => {
      expect(screen.getByTestId("review-card-bgr:bl0")).toBeInTheDocument();
    });
  });
});
