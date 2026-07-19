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
import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SeriesListItem } from "@/api/queries";
import { useAdminAuth } from "@/stores/auth";
import { SeriesCard, type SeriesSelectionProps } from "./SeriesCard";

function base(overrides: Partial<SeriesListItem>): SeriesListItem {
  return {
    id: 1,
    canonicalTitle: "Test Series",
    coverUrl: null,
    kind: "manga",
    status: "ongoing",
    year: 2020,
    description: null,
    genres: [],
    tags: [],
    metadataSource: "offline_cache",
    lastReleaseAt: Math.floor(Date.now() / 1000),
    firstSeenAt: Math.floor(Date.now() / 1000),
    releaseCount: 1,
    owned: false,
    wishlisted: false,
    ...overrides,
  };
}

// SeriesCard renders a <Link> (needs a router) and uses a mutation hook (needs
// a QueryClient).
function renderCard(
  series: SeriesListItem,
  codexSynced = false,
  selection?: SeriesSelectionProps,
) {
  const root = createRootRoute({ component: Outlet });
  const index = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: () => (
      <SeriesCard
        series={series}
        codexSynced={codexSynced}
        selection={selection}
      />
    ),
  });
  const router = createRouter({
    routeTree: root.addChildren([index]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <MantineProvider>
      <QueryClientProvider client={client}>
        {/* biome-ignore lint/suspicious/noExplicitAny: test router shape */}
        <RouterProvider router={router as any} />
      </QueryClientProvider>
    </MantineProvider>,
  );
}

describe("SeriesCard", () => {
  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("shows a manual badge for operator-authored series", async () => {
    renderCard(base({ metadataSource: "manual" }));
    expect(await screen.findByText("manual")).toBeInTheDocument();
  });

  it("omits the manual badge for provider-backed series", async () => {
    renderCard(base({ metadataSource: "offline_cache" }));
    // Wait for the card to mount, then assert the badge is absent.
    expect(await screen.findByText("Test Series")).toBeInTheDocument();
    expect(screen.queryByText("manual")).not.toBeInTheDocument();
  });

  it("renders available/total volume and chapter badges", async () => {
    renderCard(
      base({
        highestVolume: 5,
        totalVolumes: 11,
        highestChapter: 40,
        totalChapters: 97,
      }),
    );
    expect(await screen.findByText("vol 5/11")).toBeInTheDocument();
    expect(screen.getByText("ch 40/97")).toBeInTheDocument();
  });

  it("shows available count without a slash when no published total", async () => {
    renderCard(base({ highestVolume: 3, totalVolumes: null }));
    expect(await screen.findByText("vol 3")).toBeInTheDocument();
  });

  it("omits span badges when nothing is available yet", async () => {
    renderCard(base({ highestVolume: null, highestChapter: null }));
    expect(await screen.findByText("Test Series")).toBeInTheDocument();
    expect(screen.queryByText(/^vol /)).not.toBeInTheDocument();
    expect(screen.queryByText(/^ch /)).not.toBeInTheDocument();
  });

  const codexInfo = {
    status: "behind" as const,
    seriesUuid: "u1",
    deepLink: "https://codex.example.com/series/u1",
    linkKind: "auto" as const,
    syncedAt: 1700,
  };

  it("shows the Codex badge when synced and a link is present", async () => {
    renderCard(base({ codex: codexInfo }), true);
    expect(await screen.findByTestId("codex-badge-behind")).toBeInTheDocument();
  });

  it("suppresses the Codex badge before the first sync (codexSynced=false)", async () => {
    renderCard(base({ codex: codexInfo }), false);
    expect(await screen.findByText("Test Series")).toBeInTheDocument();
    expect(screen.queryByTestId("codex-badge-behind")).not.toBeInTheDocument();
  });

  it("shows no Codex badge when the series has no link", async () => {
    renderCard(base({ codex: undefined }), true);
    expect(await screen.findByText("Test Series")).toBeInTheDocument();
    expect(screen.queryByTestId(/^codex-badge-/)).not.toBeInTheDocument();
  });

  it("shows the wishlist clip toggle for admins", async () => {
    useAdminAuth.getState().setToken("test-token");
    renderCard(base({ wishlisted: false }));
    const toggle = await screen.findByTestId("wishlist-toggle-1");
    expect(toggle).toHaveAttribute("aria-label", "Add to wishlist");
    expect(toggle).toHaveTextContent("☆");
  });

  it("shows a filled star when the series is wishlisted", async () => {
    useAdminAuth.getState().setToken("test-token");
    renderCard(base({ wishlisted: true }));
    const toggle = await screen.findByTestId("wishlist-toggle-1");
    expect(toggle).toHaveAttribute("aria-label", "Remove from wishlist");
    expect(toggle).toHaveTextContent("★");
  });

  it("hides the wishlist clip toggle without an admin token", async () => {
    renderCard(base({ wishlisted: false }));
    expect(await screen.findByText("Test Series")).toBeInTheDocument();
    expect(screen.queryByTestId("wishlist-toggle-1")).not.toBeInTheDocument();
  });

  it("renders no selection checkbox without a selection prop", async () => {
    renderCard(base({}));
    expect(await screen.findByText("Test Series")).toBeInTheDocument();
    expect(screen.queryByTestId("series-select-1")).not.toBeInTheDocument();
  });

  it("selection checkbox toggles without navigating away", async () => {
    const onToggle = vi.fn();
    renderCard(base({}), false, {
      selected: false,
      active: false,
      onToggle,
    });
    const box = await screen.findByTestId("series-select-1");
    fireEvent.click(box);
    expect(onToggle).toHaveBeenCalledTimes(1);
    // Navigation to the detail route would unmount the card; it must stay.
    expect(screen.getByText("Test Series")).toBeInTheDocument();
  });

  it("selection checkbox forwards shift+click for range selection", async () => {
    const onToggle = vi.fn();
    renderCard(base({}), false, {
      selected: false,
      active: false,
      onToggle,
    });
    fireEvent.click(await screen.findByTestId("series-select-1"), {
      shiftKey: true,
    });
    expect(onToggle).toHaveBeenCalledWith(
      expect.objectContaining({ shiftKey: true }),
    );
  });

  it("selection checkbox is hidden until hover, forced visible while a selection exists", async () => {
    renderCard(base({}), false, {
      selected: false,
      active: false,
      onToggle: () => {},
    });
    const box = await screen.findByTestId("series-select-1");
    const overlay = box.closest("[data-selection-overlay]") as HTMLElement;
    expect(overlay.style.opacity).toBe("0");
    fireEvent.mouseEnter(overlay.closest("a") as HTMLElement);
    expect(overlay.style.opacity).toBe("1");
  });

  it("selection checkbox stays visible when the page selection is active", async () => {
    renderCard(base({}), false, {
      selected: false,
      active: true,
      onToggle: () => {},
    });
    const box = await screen.findByTestId("series-select-1");
    const overlay = box.closest("[data-selection-overlay]") as HTMLElement;
    expect(overlay.style.opacity).toBe("1");
  });

  it("surfaces the full title, rating and description in a hover tooltip", async () => {
    renderCard(
      base({
        canonicalTitle: "A Very Long Clamped Title",
        rating: 8.5,
        description:
          "A sweeping synopsis the card has no room to show in full.",
      }),
    );
    const title = await screen.findByText("A Very Long Clamped Title");
    fireEvent.mouseEnter(title);
    expect(await screen.findByText("★ 8.5 / 10")).toBeInTheDocument();
    expect(
      screen.getByText(
        "A sweeping synopsis the card has no room to show in full.",
      ),
    ).toBeInTheDocument();
  });
});
