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
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ADMIN_TEST_TOKEN, resetSeries } from "@/mocks/handlers";
import { useAdminAuth } from "@/stores/auth";
import { useUiPrefs } from "@/stores/uiPrefs";
import { FeedPage } from "./FeedPage";
import { SeriesDetailPage } from "./SeriesDetailPage";

// The production router lives in src/router.tsx. For tests we re-declare a
// router instance backed by in-memory history so navigation doesn't touch
// window.history. The route components themselves are the real ones.
function makeTestRouter(initialPath: string) {
  const root = createRootRoute({ component: Outlet });
  const feed = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: FeedPage,
    validateSearch: (raw: Record<string, unknown>) => {
      const out: Record<string, unknown> = {};
      const splitList = (v: unknown): string[] | undefined => {
        if (Array.isArray(v))
          return (v as unknown[]).filter(
            (x): x is string => typeof x === "string" && x.length > 0,
          );
        if (typeof v === "string" && v.length > 0)
          return v
            .split(",")
            .map((s) => s.trim())
            .filter((s) => s.length > 0);
        return undefined;
      };
      const kind = splitList(raw.kind);
      if (kind && kind.length > 0) out.kind = kind;
      const status = splitList(raw.status);
      if (status && status.length > 0) out.status = status;
      const genres = splitList(raw.genres);
      if (genres && genres.length > 0) out.genres = genres;
      if (raw.genresMode === "all" || raw.genresMode === "any")
        out.genresMode = raw.genresMode;
      const tags = splitList(raw.tags);
      if (tags && tags.length > 0) out.tags = tags;
      if (raw.tagsMode === "all" || raw.tagsMode === "any")
        out.tagsMode = raw.tagsMode;
      if (typeof raw.owned === "string")
        out.owned =
          raw.owned === "true"
            ? true
            : raw.owned === "false"
              ? false
              : undefined;
      if (typeof raw.q === "string" && raw.q.trim()) out.q = raw.q;
      if (raw.view === "list") out.view = "list";
      else if (raw.view === "card") out.view = "card";
      return out;
    },
  });
  const detail = createRoute({
    getParentRoute: () => root,
    path: "/series/$id",
    component: SeriesDetailPage,
  });
  const router = createRouter({
    routeTree: root.addChildren([feed, detail]),
    history: createMemoryHistory({ initialEntries: [initialPath] }),
  });
  return router;
}

function renderWithProviders(initialPath = "/") {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const router = makeTestRouter(initialPath);
  return {
    router,
    ...render(
      <MantineProvider>
        <QueryClientProvider client={client}>
          {/* biome-ignore lint/suspicious/noExplicitAny: route-tree shape differs between test + prod routers */}
          <RouterProvider router={router as any} />
        </QueryClientProvider>
      </MantineProvider>,
    ),
  };
}

describe("FeedPage", () => {
  it("lists series returned by the API", async () => {
    renderWithProviders("/");
    expect(
      await screen.findByText("Chainsaw Man", undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
    expect(screen.getByText("Solo Leveling")).toBeInTheDocument();
    expect(screen.getByText("10 matches")).toBeInTheDocument();
  });

  it("filters via URL search params", async () => {
    renderWithProviders("/?kind=novel");
    expect(
      await screen.findByText(
        "Re:Zero - Starting Life in Another World",
        undefined,
        { timeout: 3000 },
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("Chainsaw Man")).not.toBeInTheDocument();
    expect(screen.getByText("1 match")).toBeInTheDocument();
  });

  it("OR-combines multiple kinds via a CSV URL search param", async () => {
    // Chainsaw Man is manga, Re:Zero is a novel, Solo Leveling is manhwa.
    renderWithProviders("/?kind=manga,novel");
    expect(
      await screen.findByText("Chainsaw Man", undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Re:Zero - Starting Life in Another World"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Solo Leveling")).not.toBeInTheDocument();
  });

  it("filters by genre via URL search param", async () => {
    renderWithProviders("/?genres=isekai");
    expect(
      await screen.findByText(
        "Re:Zero - Starting Life in Another World",
        undefined,
        { timeout: 3000 },
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("Chainsaw Man")).not.toBeInTheDocument();
    expect(screen.getByText("1 match")).toBeInTheDocument();
  });

  it("AND-combines genre and tag filters", async () => {
    // Chainsaw Man is the only series tagged "gore" inside the "action" genre.
    renderWithProviders("/?genres=action&tags=gore");
    expect(
      await screen.findByText("Chainsaw Man", undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Solo Leveling")).not.toBeInTheDocument();
    expect(screen.getByText("1 match")).toBeInTheDocument();
  });

  it("opens the save-preset modal when Save is clicked", async () => {
    renderWithProviders("/?kind=manga");
    await screen.findByText("Chainsaw Man");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/Save filter preset/i)).toBeInTheDocument();
  });

  it("filters the results when a URL q= is present", async () => {
    renderWithProviders("/?q=chainsaw");
    expect(
      await screen.findByText("Chainsaw Man", undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Solo Leveling")).not.toBeInTheDocument();
    expect(screen.getByText("1 match")).toBeInTheDocument();
  });

  it("debounces the search input and refetches with the new q", async () => {
    const { router } = renderWithProviders("/");
    await screen.findByText("Chainsaw Man");
    const input = screen.getByTestId("feed-search-input") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "solo" } });
    // Input mirrors the typed value immediately…
    expect(input.value).toBe("solo");
    // …and the URL picks it up after the debounce window.
    await waitFor(
      () => {
        expect(
          (router.state.location.search as Record<string, unknown>).q,
        ).toBe("solo");
      },
      { timeout: 1500 },
    );
    await waitFor(
      () => {
        expect(screen.getByText("1 match")).toBeInTheDocument();
      },
      { timeout: 1500 },
    );
    expect(screen.queryByText("Chainsaw Man")).not.toBeInTheDocument();
  });

  it("toggles between card and list views (persisted display preference)", async () => {
    renderWithProviders("/");
    await screen.findByText("Chainsaw Man");
    expect(screen.queryByTestId("feed-list-view")).not.toBeInTheDocument();
    const toggle = screen.getByTestId("feed-view-toggle");
    // SegmentedControl renders one input[type=radio] per option; click the
    // "List" label to flip it.
    fireEvent.click(within(toggle).getByText("List"));
    expect(
      await screen.findByTestId("feed-list-view", undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("series-row-1")).toBeInTheDocument();
  });
});

describe("FeedPage bulk selection", () => {
  beforeEach(() => {
    resetSeries();
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    useUiPrefs.getState().setView("card");
  });

  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("hides selection checkboxes without an admin token", async () => {
    useAdminAuth.getState().clear();
    renderWithProviders("/");
    expect(
      await screen.findByText("Chainsaw Man", undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("series-select-1")).not.toBeInTheDocument();
  });

  it("selecting a card shows the selection bar; bulk add-to-wishlist clips it and clears the selection", async () => {
    renderWithProviders("/");
    await screen.findByText("Chainsaw Man", undefined, { timeout: 3000 });
    expect(
      screen.queryByTestId("series-selection-bar"),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId("series-select-1"));
    const bar = await screen.findByTestId("series-selection-bar");
    expect(within(bar).getByText("1 selected")).toBeInTheDocument();

    fireEvent.click(within(bar).getByTestId("bulk-wishlist-add"));
    // Success clears the selection (bar unmounts) and the refetched list
    // shows the card's star filled.
    await waitFor(() => {
      expect(
        screen.queryByTestId("series-selection-bar"),
      ).not.toBeInTheDocument();
    });
    await waitFor(() => {
      expect(screen.getByTestId("wishlist-toggle-1")).toHaveAttribute(
        "aria-label",
        "Remove from wishlist",
      );
    });
  });

  it("shift+click selects the whole range between two cards", async () => {
    renderWithProviders("/");
    await screen.findByText("Chainsaw Man", undefined, { timeout: 3000 });
    const boxes = screen.getAllByTestId(/^series-select-/);
    expect(boxes.length).toBeGreaterThanOrEqual(4);
    fireEvent.click(boxes[0]);
    fireEvent.click(boxes[3], { shiftKey: true });
    const bar = await screen.findByTestId("series-selection-bar");
    expect(within(bar).getByText("4 selected")).toBeInTheDocument();
  });

  it("select-all-on-page selects every visible card", async () => {
    renderWithProviders("/");
    await screen.findByText("Chainsaw Man", undefined, { timeout: 3000 });
    fireEvent.click(screen.getByTestId("series-select-1"));
    const bar = await screen.findByTestId("series-selection-bar");
    fireEvent.click(within(bar).getByTestId("series-select-page"));
    expect(within(bar).getByText("10 selected")).toBeInTheDocument();
  });

  it("bulk search asks for confirmation, then launches and clears the selection", async () => {
    renderWithProviders("/");
    await screen.findByText("Chainsaw Man", undefined, { timeout: 3000 });
    fireEvent.click(screen.getByTestId("series-select-1"));
    const bar = await screen.findByTestId("series-selection-bar");

    fireEvent.click(within(bar).getByTestId("bulk-search"));
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText(/Launch 1 release search\?/),
    ).toBeInTheDocument();
    fireEvent.click(within(dialog).getByTestId("bulk-search-confirm"));
    await waitFor(() => {
      expect(
        screen.queryByTestId("series-selection-bar"),
      ).not.toBeInTheDocument();
    });
  });

  it("bulk refresh-metadata reports the outcome and clears the selection", async () => {
    renderWithProviders("/");
    await screen.findByText("Chainsaw Man", undefined, { timeout: 3000 });
    fireEvent.click(screen.getByTestId("series-select-1"));
    const bar = await screen.findByTestId("series-selection-bar");
    fireEvent.click(within(bar).getByTestId("bulk-refresh"));
    await waitFor(() => {
      expect(
        screen.queryByTestId("series-selection-bar"),
      ).not.toBeInTheDocument();
    });
  });

  it("clears the selection when the query changes", async () => {
    renderWithProviders("/");
    await screen.findByText("Chainsaw Man", undefined, { timeout: 3000 });
    fireEvent.click(screen.getByTestId("series-select-1"));
    await screen.findByTestId("series-selection-bar");
    fireEvent.change(screen.getByTestId("feed-search-input"), {
      target: { value: "solo" },
    });
    // The debounced q kicks in, the page refetches, and the selection is
    // dropped (its ids referred to the old result set).
    await waitFor(
      () => {
        expect(
          screen.queryByTestId("series-selection-bar"),
        ).not.toBeInTheDocument();
      },
      { timeout: 3000 },
    );
  });

  it("offers selection checkboxes in the list view too", async () => {
    renderWithProviders("/");
    await screen.findByText("Chainsaw Man", undefined, { timeout: 3000 });
    const toggle = screen.getByTestId("feed-view-toggle");
    fireEvent.click(within(toggle).getByText("List"));
    await screen.findByTestId("feed-list-view", undefined, { timeout: 3000 });
    const row = screen.getByTestId("series-row-1");
    fireEvent.click(within(row).getByTestId("series-select-1"));
    const bar = await screen.findByTestId("series-selection-bar");
    expect(within(bar).getByText("1 selected")).toBeInTheDocument();
  });
});

describe("FeedPage filters drawer (mobile)", () => {
  it("opens the filter drawer when the Filters button is tapped", async () => {
    renderWithProviders("/");
    await screen.findByText("Chainsaw Man", undefined, { timeout: 3000 });
    // No drawer/dialog mounted until the button is pressed.
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    fireEvent.click(await screen.findByTestId("feed-filters-button"));
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
  });

  it("shows the active-filter count on the Filters button", async () => {
    renderWithProviders("/?kind=manga");
    const button = await screen.findByTestId("feed-filters-button");
    expect(within(button).getByText("1")).toBeInTheDocument();
  });
});

describe("SeriesDetailPage", () => {
  it("renders detail and releases for a series", async () => {
    renderWithProviders("/series/1");
    await waitFor(() => {
      expect(
        screen.getByRole("heading", { name: "Chainsaw Man" }),
      ).toBeInTheDocument();
    });
    expect(
      await screen.findByText(/Chainsaw Man v01 \(Digital\)/),
    ).toBeInTheDocument();
  });
});
