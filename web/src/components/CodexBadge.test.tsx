import { MantineProvider } from "@mantine/core";
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { CodexInfo } from "@/api/queries";
import { CodexBadge, codexBorderColor } from "./CodexBadge";

function info(overrides: Partial<CodexInfo>): CodexInfo {
  return {
    status: "complete",
    seriesUuid: "uuid-1",
    deepLink: "https://codex.example.com/series/uuid-1",
    linkKind: "auto",
    syncedAt: 1700,
    volumesOwned: 8,
    ...overrides,
  };
}

function renderBadge(codex: CodexInfo, asLink = false) {
  return render(
    <MantineProvider>
      <CodexBadge codex={codex} asLink={asLink} />
    </MantineProvider>,
  );
}

describe("CodexBadge", () => {
  it("renders the complete state", () => {
    renderBadge(info({ status: "complete" }));
    expect(screen.getByTestId("codex-badge-complete")).toHaveTextContent(
      "owned",
    );
  });

  it("renders the behind state", () => {
    renderBadge(info({ status: "behind" }));
    expect(screen.getByTestId("codex-badge-behind")).toHaveTextContent(
      "behind",
    );
  });

  it("renders the present (uncertain) state as plain 'owned'", () => {
    // `present` reads as "owned" like `complete`; the currency-unknown nuance
    // lives in the tooltip / outline variant, not a confusing "owned?" label.
    renderBadge(info({ status: "present" }));
    expect(screen.getByTestId("codex-badge-present")).toHaveTextContent(
      "owned",
    );
  });

  it("renders the ignored state as a distinct 'tracking off' badge", () => {
    renderBadge(info({ status: "ignored" }));
    expect(screen.getByTestId("codex-badge-ignored")).toHaveTextContent(
      "tracking off",
    );
  });

  it("renders a real anchor to the deep link when asLink", () => {
    renderBadge(
      info({ deepLink: "https://codex.example.com/series/xyz" }),
      true,
    );
    const badge = screen.getByTestId("codex-badge-complete");
    expect(badge.tagName).toBe("A");
    expect(badge).toHaveAttribute(
      "href",
      "https://codex.example.com/series/xyz",
    );
    expect(badge).toHaveAttribute("target", "_blank");
  });

  it("accents the tile border only for the actionable `behind` state", () => {
    // `behind` is the one state worth acting on, so it (and only it) gets a
    // border color; already-handled series stay quiet.
    expect(codexBorderColor("behind")).toMatch(/^var\(--mantine-color-/);
    expect(codexBorderColor("complete")).toBeNull();
    expect(codexBorderColor("present")).toBeNull();
    expect(codexBorderColor("ignored")).toBeNull();
  });

  it("opens the deep link via window.open when not a link (inside a card)", () => {
    const openSpy = vi
      .spyOn(window, "open")
      .mockImplementation(() => null as unknown as Window);
    renderBadge(info({ deepLink: "https://codex.example.com/series/abc" }));
    const badge = screen.getByTestId("codex-badge-complete");
    expect(badge.tagName).not.toBe("A");
    badge.click();
    expect(openSpy).toHaveBeenCalledWith(
      "https://codex.example.com/series/abc",
      "_blank",
      "noopener,noreferrer",
    );
    openSpy.mockRestore();
  });
});
