import { MantineProvider } from "@mantine/core";
import { Notifications } from "@mantine/notifications";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { HttpResponse, http } from "msw";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ReleaseDto } from "@/api/queries";
import { ADMIN_TEST_TOKEN } from "@/mocks/handlers";
import { server } from "@/mocks/server";
import { useAdminAuth } from "@/stores/auth";
import { SendToClientButton, SentBadge } from "./SendToClientButton";

function makeRelease(overrides: Partial<ReleaseDto> = {}): ReleaseDto {
  return {
    id: "nyaa:1",
    sourceKind: "nyaa",
    sourceName: "feed",
    externalId: "1",
    title: "Chainsaw Man v01",
    link: "https://nyaa.si/view/1",
    magnet: "magnet:?xt=urn:btih:abc",
    torrentUrl: "https://nyaa.si/download/1.torrent",
    files: [],
    formats: [],
    postedAt: 1700,
    observedAt: 1700,
    resolutionStatus: "unresolved",
    resolutionAttempts: 0,
    ...overrides,
  };
}

function renderButton(release: ReleaseDto) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <MantineProvider>
      <Notifications />
      <QueryClientProvider client={client}>
        <SendToClientButton release={release} />
      </QueryClientProvider>
    </MantineProvider>,
  );
}

describe("SendToClientButton", () => {
  beforeEach(() => {
    useAdminAuth.getState().clear();
  });
  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("renders nothing for an anonymous (non-admin) session", () => {
    renderButton(makeRelease());
    expect(
      screen.queryByTestId("send-to-client-nyaa:1"),
    ).not.toBeInTheDocument();
  });

  it("renders nothing when the integration is disabled", async () => {
    server.use(
      http.get("/api/v1/download/status", () =>
        HttpResponse.json({ enabled: false }),
      ),
    );
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderButton(makeRelease());
    // Give the status query a tick to resolve, then assert still hidden.
    await waitFor(() => {
      expect(
        screen.queryByTestId("send-to-client-nyaa:1"),
      ).not.toBeInTheDocument();
    });
  });

  it("renders nothing when the release has neither a magnet nor a torrent", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderButton(makeRelease({ magnet: null, torrentUrl: null }));
    // The status query still resolves enabled, so a missing source is the only
    // reason to hide. Wait a tick then assert hidden.
    await waitFor(() => {
      expect(
        screen.queryByTestId("send-to-client-nyaa:1"),
      ).not.toBeInTheDocument();
    });
  });

  it("shows the button for an admin once the integration is enabled", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderButton(makeRelease());
    expect(
      await screen.findByTestId("send-to-client-nyaa:1"),
    ).toBeInTheDocument();
  });

  it("sends with config defaults on a one-click and toasts success", async () => {
    // The default mock 404s for a release not in any list; intercept so the
    // one-click resolves regardless of list membership.
    server.use(
      http.post("/api/v1/releases/:id/send-to-client", () =>
        HttpResponse.json(makeRelease({ sentToClientAt: 1700 })),
      ),
    );
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderButton(makeRelease());
    const btn = await screen.findByTestId("send-to-client-nyaa:1");
    fireEvent.click(btn);
    expect(
      await screen.findByText(/Sent to torrent client/, undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
  });

  it("sends per-send overrides from the popover", async () => {
    let received: unknown;
    server.use(
      http.post("/api/v1/releases/:id/send-to-client", async ({ request }) => {
        received = await request.json();
        return HttpResponse.json(makeRelease({ sentToClientAt: 1700 }));
      }),
    );
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderButton(makeRelease());
    // Open the override popover via the caret.
    fireEvent.click(await screen.findByTestId("send-options-nyaa:1"));
    const labelInput = await screen.findByTestId("send-label-nyaa:1");
    fireEvent.change(labelInput, { target: { value: "shonen" } });
    fireEvent.click(screen.getByTestId("send-confirm-nyaa:1"));
    await waitFor(() => {
      expect(received).toMatchObject({ label: "shonen", start: true });
    });
  });
});

describe("SentBadge", () => {
  function renderBadge(release: ReleaseDto) {
    return render(
      <MantineProvider>
        <SentBadge release={release} />
      </MantineProvider>,
    );
  }

  it("renders nothing when the release was never sent", () => {
    renderBadge(makeRelease());
    expect(screen.queryByTestId("sent-badge-nyaa:1")).not.toBeInTheDocument();
  });

  it("renders a Sent badge once the release has a sent timestamp", () => {
    renderBadge(
      makeRelease({ sentToClientAt: 1700, sentToClientLabel: "manga" }),
    );
    expect(screen.getByTestId("sent-badge-nyaa:1")).toHaveTextContent("Sent");
  });
});
