import { MantineProvider } from "@mantine/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useAdminAuth } from "@/stores/auth";
import type { FilterSearch } from "@/stores/filters";
import { FilterPanel } from "./FilterPanel";

function renderPanel(
  search: FilterSearch = {},
  onChange: (next: FilterSearch) => void = () => {},
) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <MantineProvider>
      <QueryClientProvider client={client}>
        <FilterPanel search={search} onChange={onChange} />
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

  it("offers the ignored option and emits codexStatus=[ignored]", async () => {
    useAdminAuth.getState().setToken("test-admin-token");
    const onChange = vi.fn();
    renderPanel({}, onChange);
    // The test id sits on the MultiSelect's input (role combobox); clicking it
    // opens the dropdown (Mantine renders options in a portal).
    const control = await screen.findByTestId("filter-codex-status");
    fireEvent.click(control);
    fireEvent.click(await screen.findByText("Owned — tracking off"));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ codexStatus: ["ignored"], page: 1 }),
    );
  });

  it("OR-combines multiple selections (missing + behind)", async () => {
    useAdminAuth.getState().setToken("test-admin-token");
    const onChange = vi.fn();
    // Start with one selected so adding a second exercises the multi path.
    renderPanel({ codexStatus: ["missing"] }, onChange);
    const control = await screen.findByTestId("filter-codex-status");
    fireEvent.click(control);
    fireEvent.click(await screen.findByText("Owned — behind"));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ codexStatus: ["missing", "behind"], page: 1 }),
    );
  });
});

describe("FilterPanel — manual/auto source filter", () => {
  afterEach(() => useAdminAuth.getState().clear());

  it("selecting Manual emits metadataSource=manual", async () => {
    const onChange = vi.fn();
    renderPanel({}, onChange);
    const control = await screen.findByTestId("filter-metadata-source");
    fireEvent.click(within(control).getByText("Manual"));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ metadataSource: "manual", page: 1 }),
    );
  });

  it("selecting Any clears metadataSource", async () => {
    const onChange = vi.fn();
    renderPanel({ metadataSource: "manual" }, onChange);
    const control = await screen.findByTestId("filter-metadata-source");
    fireEvent.click(within(control).getByText("Any"));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ metadataSource: undefined, page: 1 }),
    );
  });
});
