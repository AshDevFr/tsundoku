import { MantineProvider } from "@mantine/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { useAdminAuth } from "@/stores/auth";
import { FilterPanel } from "./FilterPanel";

function renderPanel() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <MantineProvider>
      <QueryClientProvider client={client}>
        <FilterPanel search={{}} onChange={() => {}} />
      </QueryClientProvider>
    </MantineProvider>,
  );
}

describe("FilterPanel — Codex filter visibility", () => {
  afterEach(() => useAdminAuth.getState().clear());

  it("hides the Codex filter for non-admins (no token)", async () => {
    useAdminAuth.getState().clear();
    renderPanel();
    // The panel mounts (Filters heading present) but the admin-only control
    // is absent.
    expect(await screen.findByText("Filters")).toBeInTheDocument();
    expect(screen.queryByTestId("filter-codex-status")).not.toBeInTheDocument();
  });

  it("shows the Codex filter when an admin token is set", async () => {
    useAdminAuth.getState().setToken("test-admin-token");
    renderPanel();
    await waitFor(() =>
      expect(screen.getByTestId("filter-codex-status")).toBeInTheDocument(),
    );
  });
});
