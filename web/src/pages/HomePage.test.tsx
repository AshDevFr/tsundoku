import { MantineProvider } from "@mantine/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";
import { HomePage } from "./HomePage";

function renderWithProviders(ui: ReactNode) {
  const client = new QueryClient();
  return render(
    <MantineProvider>
      <QueryClientProvider client={client}>{ui}</QueryClientProvider>
    </MantineProvider>,
  );
}

describe("HomePage", () => {
  it("renders backend health from the mocked API", async () => {
    renderWithProviders(<HomePage />);
    await waitFor(() => {
      expect(screen.getByText(/Backend status: ok/)).toBeInTheDocument();
    });
  });
});
