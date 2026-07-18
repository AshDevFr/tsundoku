import { MantineProvider } from "@mantine/core";
import { Notifications } from "@mantine/notifications";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { HttpResponse, http } from "msw";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  ADMIN_TEST_TOKEN,
  resetSearch,
  seedSearchEntries,
} from "@/mocks/handlers";
import { server } from "@/mocks/server";
import { useAdminAuth } from "@/stores/auth";
import { AdminSourcesListPage } from "./SourcesList";

function renderPage() {
  // No sources in these tests (SourceCard links need a router); the page
  // still renders its search-endpoints section below the empty state.
  server.use(
    http.get("/api/v1/sources", () => HttpResponse.json({ items: [] })),
  );
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <MantineProvider>
      <Notifications />
      <QueryClientProvider client={client}>
        <AdminSourcesListPage />
      </QueryClientProvider>
    </MantineProvider>,
  );
}

describe("AdminSourcesListPage search endpoints", () => {
  beforeEach(() => {
    resetSearch();
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
  });
  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("lists the configured search endpoints with the default badge", async () => {
    renderPage();
    expect(
      await screen.findByTestId("search-endpoints-section"),
    ).toBeInTheDocument();

    const eng = screen.getByTestId("search-endpoint-Nyaa Literature - Eng");
    expect(eng).toHaveTextContent("default");
    expect(eng).toHaveTextContent("https://nyaa.si/?f=0&c=3_1");
    expect(eng).toHaveTextContent("nyaa");

    const raw = screen.getByTestId("search-endpoint-Nyaa Literature - Raw");
    expect(raw).not.toHaveTextContent("default");
    expect(raw).toHaveTextContent("https://nyaa.si/?f=0&c=3_3");
  });

  it("hides the section when no search entries are configured", async () => {
    seedSearchEntries([]);
    renderPage();
    // Wait for the page to settle (sources empty-state renders), then
    // assert the section never appeared.
    await screen.findByText(/No sources registered/);
    expect(
      screen.queryByTestId("search-endpoints-section"),
    ).not.toBeInTheDocument();
  });
});
