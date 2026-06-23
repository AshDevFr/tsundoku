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

describe("FilterPanel — wishlist filter", () => {
  afterEach(() => useAdminAuth.getState().clear());

  it("hides the wishlist filter for non-admins", async () => {
    useAdminAuth.getState().clear();
    renderPanel();
    expect(await screen.findByText("Filters")).toBeInTheDocument();
    expect(screen.queryByTestId("filter-wishlisted")).not.toBeInTheDocument();
  });

  it("selecting Wishlisted emits wishlisted=true", async () => {
    useAdminAuth.getState().setToken("test-admin-token");
    const onChange = vi.fn();
    renderPanel({}, onChange);
    const control = await screen.findByTestId("filter-wishlisted");
    fireEvent.click(within(control).getByText("Wishlisted"));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ wishlisted: true, page: 1 }),
    );
  });

  it("selecting Any clears the wishlist filter", async () => {
    useAdminAuth.getState().setToken("test-admin-token");
    const onChange = vi.fn();
    renderPanel({ wishlisted: true }, onChange);
    const control = await screen.findByTestId("filter-wishlisted");
    fireEvent.click(within(control).getByText("Any"));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ wishlisted: undefined, page: 1 }),
    );
  });
});

describe("FilterPanel — sort", () => {
  afterEach(() => useAdminAuth.getState().clear());

  it("selecting Publication date emits sort=published_start_date", async () => {
    const onChange = vi.fn();
    renderPanel({}, onChange);
    fireEvent.click(await screen.findByTestId("filter-sort"));
    fireEvent.click(await screen.findByText("Publication date"));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ sort: "published_start_date", page: 1 }),
    );
  });
});

describe("FilterPanel — kind / status multi-select", () => {
  afterEach(() => useAdminAuth.getState().clear());

  it("emits kind as an array when an option is picked", async () => {
    const onChange = vi.fn();
    renderPanel({}, onChange);
    const control = await screen.findByTestId("filter-kind");
    fireEvent.click(control);
    fireEvent.click(await screen.findByText("manhwa"));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ kind: ["manhwa"], page: 1 }),
    );
  });

  it("OR-combines multiple status selections", async () => {
    const onChange = vi.fn();
    // Start with one selected so adding a second exercises the multi path.
    renderPanel({ status: ["ongoing"] }, onChange);
    const control = await screen.findByTestId("filter-status");
    fireEvent.click(control);
    fireEvent.click(await screen.findByText("completed"));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ status: ["ongoing", "completed"], page: 1 }),
    );
  });
});

describe("FilterPanel — sources filter (admin-only)", () => {
  afterEach(() => useAdminAuth.getState().clear());

  it("hides the sources filter for non-admins", async () => {
    useAdminAuth.getState().clear();
    renderPanel();
    expect(await screen.findByText("Filters")).toBeInTheDocument();
    expect(screen.queryByTestId("filter-sources")).not.toBeInTheDocument();
  });

  it("shows the sources filter and emits the feed name on select", async () => {
    useAdminAuth.getState().setToken("test-admin-token");
    const onChange = vi.fn();
    renderPanel({}, onChange);
    const control = await screen.findByTestId("filter-sources");
    fireEvent.click(control);
    // Options carry the per-feed series count in the label; selecting one
    // emits just the bare feed name (the value), OR-combined like kind/status.
    fireEvent.click(await screen.findByText("english-manga-trusted (3)"));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ sources: ["english-manga-trusted"], page: 1 }),
    );
  });
});

describe("FilterPanel — search descriptions toggle", () => {
  afterEach(() => useAdminAuth.getState().clear());

  it("is off by default and visible to everyone", async () => {
    renderPanel();
    const toggle = await screen.findByTestId("feed-search-descriptions-toggle");
    expect(toggle).not.toBeChecked();
  });

  it("turning it on emits searchDescriptions=true", async () => {
    const onChange = vi.fn();
    renderPanel({}, onChange);
    const toggle = await screen.findByTestId("feed-search-descriptions-toggle");
    fireEvent.click(toggle);
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ searchDescriptions: true, page: 1 }),
    );
  });

  it("turning it off clears the flag (undefined, not false)", async () => {
    const onChange = vi.fn();
    renderPanel({ searchDescriptions: true }, onChange);
    const toggle = await screen.findByTestId("feed-search-descriptions-toggle");
    expect(toggle).toBeChecked();
    fireEvent.click(toggle);
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ searchDescriptions: undefined, page: 1 }),
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
