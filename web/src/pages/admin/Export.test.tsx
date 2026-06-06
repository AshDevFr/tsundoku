import { MantineProvider } from "@mantine/core";
import { Notifications } from "@mantine/notifications";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { HttpResponse, http } from "msw";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ADMIN_TEST_TOKEN } from "@/mocks/handlers";
import { server } from "@/mocks/server";
import { useAdminAuth } from "@/stores/auth";
import { AdminExportPage } from "./Export";

function renderPage() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <MantineProvider>
      <Notifications />
      <QueryClientProvider client={client}>
        <AdminExportPage />
      </QueryClientProvider>
    </MantineProvider>,
  );
}

/// The `<input>` inside a field checkbox (the `data-testid` sits on the
/// Mantine root wrapper). Using testids avoids label collisions with the
/// like-named filter controls (Type / Status / Genres / Tags).
function fieldInput(key: string): HTMLInputElement {
  // Mantine forwards `data-testid` onto the checkbox `<input>` itself; fall
  // back to a descendant input if a wrapper carries it instead.
  const el = screen.getByTestId(`export-field-${key}`);
  return (
    el.tagName === "INPUT" ? el : el.querySelector("input")
  ) as HTMLInputElement;
}

describe("AdminExportPage", () => {
  beforeEach(() => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
  });
  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("locks the title field on and defaults bookkeeping fields off", () => {
    renderPage();
    const title = fieldInput("canonicalTitle");
    expect(title).toBeChecked();
    expect(title).toBeDisabled();
    // Default-off bookkeeping fields.
    expect(fieldInput("id")).not.toBeChecked();
    expect(fieldInput("coverUrl")).not.toBeChecked();
    // Default-on agent-relevant fields.
    expect(fieldInput("genres")).toBeChecked();
    expect(fieldInput("codexStatus")).toBeChecked();
  });

  it("disables the include-releases switch for CSV", () => {
    renderPage();
    const sw = screen.getByLabelText(
      /include linked releases/i,
    ) as HTMLInputElement;
    expect(sw).not.toBeDisabled();
    fireEvent.click(screen.getByText("CSV"));
    expect(screen.getByLabelText(/include linked releases/i)).toBeDisabled();
  });

  it("select-all checks every field; clear leaves only the title", () => {
    renderPage();
    fireEvent.click(screen.getByTestId("export-select-all"));
    expect(fieldInput("id")).toBeChecked();
    expect(fieldInput("coverUrl")).toBeChecked();

    fireEvent.click(screen.getByTestId("export-clear"));
    expect(fieldInput("id")).not.toBeChecked();
    expect(fieldInput("genres")).not.toBeChecked();
    // Title is always on.
    expect(fieldInput("canonicalTitle")).toBeChecked();
  });

  it("exports with the selected format and fields, and notifies on success", async () => {
    let captured = "";
    server.use(
      http.get("/api/v1/series/export", ({ request }) => {
        captured = request.url;
        return new HttpResponse("[]", {
          headers: {
            "content-type": "application/json; charset=utf-8",
            "content-disposition":
              'attachment; filename="tsundoku-series-export-2026-06-08.json"',
          },
        });
      }),
    );
    renderPage();
    fireEvent.click(screen.getByTestId("export-button"));

    await waitFor(() => expect(captured).not.toBe(""));
    const sp = new URL(captured).searchParams;
    expect(sp.get("format")).toBe("json");
    const fields = sp.get("fields")?.split(",") ?? [];
    expect(fields).toContain("canonicalTitle");
    expect(fields).toContain("genres");
    // A default-off field is not in the request.
    expect(fields).not.toContain("id");

    expect(await screen.findByText(/Export started/i)).toBeInTheDocument();
  });

  it("surfaces a failed export", async () => {
    server.use(
      http.get(
        "/api/v1/series/export",
        () => new HttpResponse(null, { status: 500 }),
      ),
    );
    renderPage();
    fireEvent.click(screen.getByTestId("export-button"));
    // Match the notification title exactly (the description is "Export failed
    // (500)", which would otherwise also match a loose regex).
    expect(await screen.findByText("Export failed")).toBeInTheDocument();
  });
});
