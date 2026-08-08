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
import { HttpResponse, http } from "msw";
import { beforeEach, describe, expect, it } from "vitest";
import { ADMIN_TEST_TOKEN, resetSeries } from "@/mocks/handlers";
import { server } from "@/mocks/server";
import { useAdminAuth } from "@/stores/auth";
import { AdminReleasesPage } from "./Releases";

function renderReleases() {
  const root = createRootRoute({ component: Outlet });
  const releases = createRoute({
    getParentRoute: () => root,
    path: "/admin/releases",
    component: AdminReleasesPage,
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
    routeTree: root.addChildren([releases, detail]),
    history: createMemoryHistory({ initialEntries: ["/admin/releases"] }),
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <MantineProvider>
      <QueryClientProvider client={client}>
        {/* biome-ignore lint/suspicious/noExplicitAny: route-tree shape differs between test + prod routers */}
        <RouterProvider router={router as any} />
      </QueryClientProvider>
    </MantineProvider>,
  );
}

async function searchFor(value: string) {
  fireEvent.change(await screen.findByTestId("releases-q"), {
    target: { value },
  });
  fireEvent.click(screen.getByTestId("releases-apply"));
}

describe("AdminReleasesPage", () => {
  beforeEach(() => {
    resetSeries();
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
  });

  it("lists releases with their status and linked series", async () => {
    renderReleases();
    expect(await screen.findByTestId("release-row-nyaa:111")).toBeVisible();
    const row = screen.getByTestId("release-row-nyaa:111");
    expect(row).toHaveTextContent("resolved");
    expect(row).toHaveTextContent("series #1");
  });

  it("navigates to the linked series, answering 'where did it go?'", async () => {
    renderReleases();
    const row = await screen.findByTestId("release-row-nyaa:111");
    fireEvent.click(within(row).getByText("series #1"));
    await waitFor(() =>
      expect(screen.getByText("detail-page-1")).toBeInTheDocument(),
    );
  });

  it("resolves a pasted post URL to the one release", async () => {
    renderReleases();
    await searchFor("https://nyaa.si/view/113");
    await waitFor(() => {
      expect(screen.getByTestId("releases-results").children).toHaveLength(1);
    });
    expect(screen.getByTestId("release-row-nyaa:113")).toBeVisible();
  });

  it("matches title words in any order", async () => {
    renderReleases();
    await searchFor("v03 Chainsaw");
    await waitFor(() => {
      expect(screen.getByTestId("release-row-nyaa:113")).toBeVisible();
    });
  });

  it("explains a miss rather than showing an empty page", async () => {
    renderReleases();
    await searchFor("https://nyaa.si/view/999999");
    expect(await screen.findByText(/Nothing matched/i)).toBeInTheDocument();
  });

  // The page exists largely so `rejected` releases are reachable at all — they
  // have no other surface in the UI.
  it("can surface rejected releases", async () => {
    server.use(
      http.get("/api/v1/releases", () =>
        HttpResponse.json({
          items: [
            {
              id: "nyaa:900",
              sourceKind: "nyaa",
              sourceName: "feed",
              externalId: "900",
              title: "Rejected Thing v01",
              link: "https://nyaa.si/view/900",
              magnet: null,
              torrentUrl: null,
              ddlUrl: null,
              infoHash: null,
              sizeBytes: null,
              files: [],
              formats: [],
              postedAt: 1,
              observedAt: 2,
              seriesId: null,
              resolutionPath: "rejected",
              resolutionConfidence: null,
              resolutionStatus: "rejected",
              resolutionAttempts: 1,
              lastResolveAttemptAt: 2,
            },
          ],
          page: 1,
          pageSize: 50,
          total: 1,
        }),
      ),
    );
    renderReleases();
    const row = await screen.findByTestId("release-row-nyaa:900");
    expect(row).toHaveTextContent("rejected");
    expect(row).toHaveTextContent("no series");
  });

  it("does not query until the search is submitted", async () => {
    let calls = 0;
    server.use(
      http.get("/api/v1/releases", () => {
        calls += 1;
        return HttpResponse.json({
          items: [],
          page: 1,
          pageSize: 50,
          total: 0,
        });
      }),
    );
    renderReleases();
    await waitFor(() => expect(calls).toBe(1)); // the initial unfiltered load
    const input = await screen.findByTestId("releases-q");
    for (const value of ["C", "Ch", "Cha"]) {
      fireEvent.change(input, { target: { value } });
    }
    await waitFor(() => expect(calls).toBe(1));
  });
});
