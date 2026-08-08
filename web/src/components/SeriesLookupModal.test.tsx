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
import { HttpResponse, http } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { resetSeries } from "@/mocks/handlers";
import { server } from "@/mocks/server";
import { SeriesLookupModal } from "./SeriesLookupModal";

/// Mount the modal inside a real router so navigation on a hit is observable,
/// mirroring the SeriesLookupPage harness.
function renderModal(onClose = vi.fn()) {
  const root = createRootRoute({ component: Outlet });
  const home = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: () => <SeriesLookupModal opened onClose={onClose} />,
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
    routeTree: root.addChildren([home, detail]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
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
  return { ...view, router, onClose };
}

/// The router resolves its initial route asynchronously, so the modal's portal
/// only exists after a tick — every query into it has to be the async form.
async function submit(value: string) {
  fireEvent.change(await screen.findByTestId("series-lookup-input"), {
    target: { value },
  });
  fireEvent.click(screen.getByRole("button", { name: "Go" }));
}

describe("SeriesLookupModal", () => {
  beforeEach(() => resetSeries());

  it("navigates straight to the series on a single match", async () => {
    const { onClose } = renderModal();
    // Fixture ids are mangabaka-shaped (`id * 1111`), so 1111 -> series 1.
    await submit("1111");
    await waitFor(() => {
      expect(screen.getByText("detail-page-1")).toBeInTheDocument();
    });
    expect(onClose).toHaveBeenCalled();
  });

  it("resolves a pasted series URL without needing the provider dropdown", async () => {
    renderModal();
    await submit("https://mangabaka.dev/1111");
    await waitFor(() => {
      expect(screen.getByText("detail-page-1")).toBeInTheDocument();
    });
  });

  it("explains a miss instead of erroring", async () => {
    renderModal();
    await submit("404404");
    expect(
      await screen.findByText(/No series carries that ID/i),
    ).toBeInTheDocument();
  });

  // A bare id can legitimately belong to several providers, so the modal must
  // ask rather than pick one. Overridden here because the shared fixture
  // handler only models mangabaka-shaped ids.
  it("offers a pick list when one id maps to several providers", async () => {
    server.use(
      http.get("/api/v1/series/lookup", () =>
        HttpResponse.json({
          matches: [
            {
              seriesId: 7,
              provider: "mal",
              externalId: "1329",
              canonicalTitle: "MAL Title",
            },
            {
              seriesId: 9,
              provider: "mangabaka",
              externalId: "1329",
              canonicalTitle: "MangaBaka Title",
            },
          ],
        }),
      ),
    );
    renderModal();
    await submit("1329");

    const list = await screen.findByTestId("series-lookup-matches");
    expect(list).toHaveTextContent("mal:1329");
    expect(list).toHaveTextContent("mangabaka:1329");
    // Still on the modal — no silent guess between the two.
    expect(screen.queryByText(/^detail-page-/)).not.toBeInTheDocument();

    fireEvent.click(screen.getByText("MangaBaka Title"));
    await waitFor(() => {
      expect(screen.getByText("detail-page-9")).toBeInTheDocument();
    });
  });

  it("does not query until submitted", async () => {
    let calls = 0;
    server.use(
      http.get("/api/v1/series/lookup", () => {
        calls += 1;
        return HttpResponse.json({ matches: [] });
      }),
    );
    renderModal();
    const input = await screen.findByTestId("series-lookup-input");
    for (const value of ["1", "13", "132"]) {
      fireEvent.change(input, { target: { value } });
    }
    await waitFor(() => expect(calls).toBe(0));
  });
});
