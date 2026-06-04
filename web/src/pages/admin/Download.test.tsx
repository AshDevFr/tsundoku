import { MantineProvider } from "@mantine/core";
import { Notifications } from "@mantine/notifications";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import { HttpResponse, http } from "msw";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ADMIN_TEST_TOKEN } from "@/mocks/handlers";
import { server } from "@/mocks/server";
import { useAdminAuth } from "@/stores/auth";
import { AdminDownloadPage } from "./Download";

function renderPage() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <MantineProvider>
      <Notifications />
      <QueryClientProvider client={client}>
        <AdminDownloadPage />
      </QueryClientProvider>
    </MantineProvider>,
  );
}

describe("AdminDownloadPage", () => {
  beforeEach(() => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
  });
  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("shows the disabled notice when the integration is off", async () => {
    server.use(
      http.get("/api/v1/download/status", () =>
        HttpResponse.json({
          enabled: false,
          hasCredentials: false,
          defaultStart: true,
          preferTorrentFile: true,
          reachable: false,
          recentChecks: [],
          recentSends: [],
        }),
      ),
    );
    renderPage();
    expect(await screen.findByTestId("download-disabled")).toBeInTheDocument();
    expect(screen.queryByTestId("download-card")).not.toBeInTheDocument();
  });

  it("renders connection info and the reachable badge when enabled", async () => {
    renderPage();
    expect(await screen.findByTestId("download-card")).toBeInTheDocument();
    expect(screen.getByTestId("download-reachable")).toHaveTextContent(
      "Reachable",
    );
    expect(
      screen.getByText("https://box.example.com/rutorrent"),
    ).toBeInTheDocument();
  });

  it("renders the recent-checks history", async () => {
    renderPage();
    await screen.findByTestId("download-card");
    expect(screen.getByText("Recent checks")).toBeInTheDocument();
  });

  it("names the sent release and marks the label as a label", async () => {
    renderPage();
    await screen.findByTestId("download-card");
    expect(screen.getByText("Recent sends")).toBeInTheDocument();
    // The release title identifies what was sent (not just the bare id).
    expect(screen.getByText("Chainsaw Man v01")).toBeInTheDocument();
    // The label is explicitly prefixed so it isn't mistaken for the source.
    expect(screen.getByText(/label: manga/)).toBeInTheDocument();
    expect(screen.getByText(/via torrent/)).toBeInTheDocument();
  });

  it("runs the connection test and toasts the unreachable result", async () => {
    renderPage();
    const btn = await screen.findByTestId("download-test");
    fireEvent.click(btn);
    expect(
      await screen.findByText(/Unreachable: connection refused/, undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
  });
});
