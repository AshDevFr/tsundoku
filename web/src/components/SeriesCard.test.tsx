import { MantineProvider } from "@mantine/core";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { SeriesListItem } from "@/api/queries";
import { SeriesCard } from "./SeriesCard";

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
    ...overrides,
  };
}

// SeriesCard renders a <Link>, so it needs a router context to mount.
function renderCard(series: SeriesListItem, codexSynced = false) {
  const root = createRootRoute({ component: Outlet });
  const index = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: () => <SeriesCard series={series} codexSynced={codexSynced} />,
  });
  const router = createRouter({
    routeTree: root.addChildren([index]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  return render(
    <MantineProvider>
      {/* biome-ignore lint/suspicious/noExplicitAny: test router shape */}
      <RouterProvider router={router as any} />
    </MantineProvider>,
  );
}

describe("SeriesCard", () => {
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
});
