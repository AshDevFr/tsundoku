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
import { describe, expect, it } from "vitest";
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
      if (typeof raw.kind === "string") out.kind = raw.kind;
      if (typeof raw.status === "string") out.status = raw.status;
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
    expect(screen.getByText("9 matches")).toBeInTheDocument();
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

  it("toggles between card and list views via the URL view= param", async () => {
    const { router } = renderWithProviders("/");
    await screen.findByText("Chainsaw Man");
    expect(screen.queryByTestId("feed-list-view")).not.toBeInTheDocument();
    const toggle = screen.getByTestId("feed-view-toggle");
    // SegmentedControl renders one input[type=radio] per option; click the
    // "List" label to flip it.
    fireEvent.click(within(toggle).getByText("List"));
    await waitFor(() => {
      expect(
        (router.state.location.search as Record<string, unknown>).view,
      ).toBe("list");
    });
    expect(
      await screen.findByTestId("feed-list-view", undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("series-row-1")).toBeInTheDocument();
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
