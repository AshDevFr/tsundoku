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
      if (typeof raw.owned === "string")
        out.owned =
          raw.owned === "true"
            ? true
            : raw.owned === "false"
              ? false
              : undefined;
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
    expect(screen.getByText("3 matches")).toBeInTheDocument();
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

  it("opens the save-preset modal when Save is clicked", async () => {
    renderWithProviders("/?kind=manga");
    await screen.findByText("Chainsaw Man");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/Save filter preset/i)).toBeInTheDocument();
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
