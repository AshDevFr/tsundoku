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
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { resetSeries } from "@/mocks/handlers";
import { SeriesLookupPage, validateLookupSearch } from "./SeriesLookupPage";

// Rebuild the `/series/lookup` route locally, like the SeriesDetailPage tests
// do for `/series/$id`. The stub detail route renders a marker so a successful
// resolve is observable; the stub feed route lets the miss state's links
// resolve.
function renderLookup(initialEntry: string) {
  const root = createRootRoute({ component: Outlet });
  const feed = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: () => null,
  });
  const lookup = createRoute({
    getParentRoute: () => root,
    path: "/series/lookup",
    component: SeriesLookupPage,
    validateSearch: validateLookupSearch,
  });
  const detail = createRoute({
    getParentRoute: () => root,
    path: "/series/$id",
    component: function DetailMarker() {
      const { id } = detail.useParams();
      return <div>detail-page-{id}</div>;
    },
  });
  const router = createRouter({
    routeTree: root.addChildren([feed, lookup, detail]),
    history: createMemoryHistory({ initialEntries: [initialEntry] }),
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = render(
    <MantineProvider>
      <QueryClientProvider client={client}>
        {/* biome-ignore lint/suspicious/noExplicitAny: route-tree shape differs between test + prod routers */}
        <RouterProvider router={router as any} />
      </QueryClientProvider>
    </MantineProvider>,
  );
  return { ...view, router };
}

describe("validateLookupSearch", () => {
  it("keeps a numeric id as a number so the URL round-trips unquoted", () => {
    // The default TanStack search codec JSON-parses `id=4623` to a number;
    // coercing it to a string here would make stringifySearch re-serialize
    // the address bar as id=%224623%22.
    expect(validateLookupSearch({ id: 4623 })).toEqual({ id: 4623 });
  });

  it("trims a string id and keeps it a string", () => {
    expect(validateLookupSearch({ id: " 6z1uqw7 " })).toEqual({
      id: "6z1uqw7",
    });
  });
});

describe("SeriesLookupPage", () => {
  beforeEach(() => {
    resetSeries();
  });

  it("resolves a known external id and replace-navigates to the detail page", async () => {
    // Series 1's synthetic mangabaka id in the mock catalog is 1111.
    const { router } = renderLookup("/series/lookup?source=mangabaka&id=1111");
    await screen.findByText("detail-page-1");
    expect(router.state.location.pathname).toBe("/series/1");
    // Replace-navigation: the resolver URL must not linger in history, so
    // Back from the detail page skips it.
    await router.history.back();
    await waitFor(() =>
      expect(router.state.location.pathname).not.toBe("/series/lookup"),
    );
  });

  it("renders the not-found state for an unknown mapping", async () => {
    renderLookup("/series/lookup?source=mangabaka&id=999999");
    await screen.findByText(/isn't in tsundoku/i);
    expect(screen.getByText(/mangabaka:999999/)).toBeInTheDocument();
    // No title param, so no feed-search shortcut; the plain feed link remains.
    expect(
      screen.queryByRole("link", { name: /Search the feed/i }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: /Go to the feed/i }),
    ).toBeInTheDocument();
  });

  it("offers a feed search for the title on a miss when one is provided", async () => {
    renderLookup(
      "/series/lookup?source=mangabaka&id=999999&title=Chainsaw%20Man",
    );
    await screen.findByText(/isn't in tsundoku/i);
    const search = screen.getByRole("link", { name: /Search the feed/i });
    const href = search.getAttribute("href") ?? "";
    expect(href).toMatch(/^\/\?/);
    expect(href).toContain("q=Chainsaw");
  });

  it("flags an incomplete link without calling the API", async () => {
    renderLookup("/series/lookup?source=mangabaka");
    await screen.findByText(/incomplete/i);
    expect(
      screen.getByRole("link", { name: /Go to the feed/i }),
    ).toBeInTheDocument();
  });
});
