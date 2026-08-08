import { MantineProvider } from "@mantine/core";
import { Notifications } from "@mantine/notifications";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { HttpResponse, http } from "msw";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ADMIN_TEST_TOKEN } from "@/mocks/handlers";
import { server } from "@/mocks/server";
import { useAdminAuth } from "@/stores/auth";
import { AdminMaintenancePage } from "./Maintenance";

function renderPage() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <MantineProvider>
      <Notifications />
      <QueryClientProvider client={client}>
        <AdminMaintenancePage />
      </QueryClientProvider>
    </MantineProvider>,
  );
}

function excludeWishlistedInput(): HTMLInputElement {
  const el = screen.getByTestId("purge-orphans-exclude-wishlisted");
  return (
    el.tagName === "INPUT" ? el : el.querySelector("input")
  ) as HTMLInputElement;
}

describe("AdminMaintenancePage — purge orphan series", () => {
  beforeEach(() => useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN));
  afterEach(() => useAdminAuth.getState().clear());

  // The safe default. Off by accident would mean a click deletes series the
  // operator deliberately clipped.
  it("excludes wishlisted series by default", async () => {
    renderPage();
    await waitFor(() => expect(excludeWishlistedInput()).toBeChecked());
  });

  it("shows the dry-run count and a sample before anything is deleted", async () => {
    renderPage();
    await waitFor(() =>
      expect(screen.getByTestId("purge-orphans-count")).toHaveTextContent(
        "2 series would be deleted",
      ),
    );
    const sample = screen.getByTestId("purge-orphans-sample");
    expect(sample).toHaveTextContent("Orphan One");
    expect(sample).toHaveTextContent("Orphan Two");
  });

  // Turning the toggle off must visibly widen the set *before* the operator
  // commits, not surprise them afterwards.
  it("re-runs the dry run when the wishlist toggle changes", async () => {
    renderPage();
    await waitFor(() =>
      expect(screen.getByTestId("purge-orphans-count")).toHaveTextContent("2"),
    );
    fireEvent.click(excludeWishlistedInput());
    await waitFor(() =>
      expect(screen.getByTestId("purge-orphans-count")).toHaveTextContent(
        "3 series would be deleted",
      ),
    );
    expect(screen.getByTestId("purge-orphans-sample")).toHaveTextContent(
      "Wishlisted Orphan",
    );
  });

  it("requires confirmation, and states the count and wishlist scope in it", async () => {
    renderPage();
    await waitFor(() =>
      expect(screen.getByTestId("purge-orphans-open")).toBeEnabled(),
    );
    fireEvent.click(screen.getByTestId("purge-orphans-open"));

    const confirm = await screen.findByTestId("purge-orphans-confirm");
    expect(confirm).toHaveTextContent("Delete 2 series");
    expect(screen.getByText(/cannot be undone/i)).toBeInTheDocument();
    expect(
      screen.getByText(/Wishlisted series are excluded/i),
    ).toBeInTheDocument();

    fireEvent.click(confirm);
    await waitFor(() =>
      expect(screen.getByText(/Orphan series purged/i)).toBeInTheDocument(),
    );
  });

  // The modal is the last chance to notice, so it must say plainly that the
  // wishlist guard is off rather than quietly using the widened set.
  it("warns in the modal when wishlisted series are included", async () => {
    renderPage();
    await waitFor(() =>
      expect(screen.getByTestId("purge-orphans-count")).toHaveTextContent("2"),
    );
    fireEvent.click(excludeWishlistedInput());
    await waitFor(() =>
      expect(screen.getByTestId("purge-orphans-count")).toHaveTextContent("3"),
    );
    fireEvent.click(screen.getByTestId("purge-orphans-open"));
    expect(
      await screen.findByText(/Wishlisted series are INCLUDED/i),
    ).toBeInTheDocument();
  });

  // With nothing to delete the action should be unreachable rather than a
  // no-op the operator has to confirm.
  it("disables the purge button when nothing matches", async () => {
    server.use(
      http.get("/api/v1/maintenance/orphan-series", () =>
        HttpResponse.json({ count: 0, sample: [] }),
      ),
    );
    renderPage();
    await waitFor(() =>
      expect(screen.getByTestId("purge-orphans-count")).toHaveTextContent(
        "0 series would be deleted",
      ),
    );
    const card = screen.getByTestId("maintenance-purge-orphans-card");
    expect(within(card).getByTestId("purge-orphans-open")).toBeDisabled();
  });
});
