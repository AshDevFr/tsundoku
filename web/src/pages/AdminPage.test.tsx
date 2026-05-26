import { MantineProvider } from "@mantine/core";
import { Notifications } from "@mantine/notifications";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ADMIN_TEST_TOKEN } from "@/mocks/handlers";
import { useAdminAuth } from "@/stores/auth";
import { AdminPage } from "./AdminPage";

function makeRouter() {
  const root = createRootRoute({ component: Outlet });
  const admin = createRoute({
    getParentRoute: () => root,
    path: "/admin",
    component: AdminPage,
  });
  // /review is referenced from the admin shell as an anchor; provide a stub
  // route so RouterProvider does not blow up resolving the Link.
  const review = createRoute({
    getParentRoute: () => root,
    path: "/review",
    component: () => null,
  });
  return createRouter({
    routeTree: root.addChildren([admin, review]),
    history: createMemoryHistory({ initialEntries: ["/admin"] }),
  });
}

function renderAdmin() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const router = makeRouter();
  return render(
    <MantineProvider>
      <Notifications />
      <QueryClientProvider client={client}>
        {/* biome-ignore lint/suspicious/noExplicitAny: route-tree shape differs between test + prod routers */}
        <RouterProvider router={router as any} />
      </QueryClientProvider>
    </MantineProvider>,
  );
}

describe("AdminPage", () => {
  beforeEach(() => {
    useAdminAuth.getState().clear();
  });

  afterEach(() => {
    useAdminAuth.getState().clear();
  });

  it("requires admin token before showing the dashboard", async () => {
    renderAdmin();
    expect(await screen.findByText(/Admin sign-in/i)).toBeInTheDocument();
    expect(screen.queryByText(/Discovery sources/)).not.toBeInTheDocument();
  });

  it("renders source and provider cards once signed in", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderAdmin();

    expect(
      await screen.findByText(/Discovery sources/, undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    expect(
      await screen.findByTestId("source-card-english-manga-trusted"),
    ).toBeInTheDocument();
    expect(
      await screen.findByTestId("provider-card-mangabaka"),
    ).toBeInTheDocument();
    // Surface the cron / feed_url for the source.
    expect(screen.getByText(/\*\/30 \* \* \* \*/)).toBeInTheDocument();
    expect(screen.getByText(/nyaa\.si\/\?page=rss/)).toBeInTheDocument();
  });

  it("never renders the raw api_key value", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    const { container } = renderAdmin();
    await screen.findByTestId("provider-card-mangabaka", undefined, {
      timeout: 3000,
    });
    // The MSW response carries apiKeySet=true but no raw key; ensure nothing
    // looks like an exposed token. We assert on the `apiKey` string itself
    // not appearing (only `apiKeySet`).
    const html = container.innerHTML;
    expect(html).not.toMatch(/"apiKey"\s*:\s*"/);
    // The set/not-set badge renders instead.
    expect(screen.getByTestId("api-key-set-badge")).toHaveTextContent(/set/i);
  });

  it("dispatches the poll-all mutation", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderAdmin();
    // Wait for the underlying sources query to settle so the button drops
    // the `disabled` flag the empty-list guard applies.
    await screen.findByTestId("source-card-english-manga-trusted", undefined, {
      timeout: 3000,
    });
    const button = await screen.findByTestId("poll-all-sources");
    expect(button).not.toBeDisabled();
    fireEvent.click(button);
    await waitFor(
      () => {
        expect(
          screen.getByText(/1 triggered, 0 already running/),
        ).toBeInTheDocument();
      },
      { timeout: 3000 },
    );
  });

  it("renders resolution-outcome breakdown and review-queue card under metrics", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderAdmin();
    await screen.findByText(/Metrics/, undefined, { timeout: 3000 });
    expect(
      await screen.findByTestId("outcome-breakdown", undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    expect(await screen.findByTestId("error-kind-donut")).toBeInTheDocument();
    expect(
      await screen.findByTestId("review-queue-metrics-card"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("review-queue-depth-sparkline"),
    ).toBeInTheDocument();
  });

  it("renders the metrics section with per-source cards and a sparkline", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderAdmin();
    expect(
      await screen.findByText(/Metrics/, undefined, { timeout: 3000 }),
    ).toBeInTheDocument();
    expect(
      await screen.findByTestId(
        "metrics-card-english-manga-trusted",
        undefined,
        { timeout: 3000 },
      ),
    ).toBeInTheDocument();
    // Success-rate badge derives 92% from 11/12.
    expect(screen.getByText(/92% success/)).toBeInTheDocument();
    // Inline SVG sparkline renders for the source detail buckets.
    expect(
      await screen.findByTestId("metrics-sparkline", undefined, {
        timeout: 3000,
      }),
    ).toBeInTheDocument();
    // Provider refresh card alongside the sources.
    expect(
      screen.getByTestId("provider-metrics-card-mangabaka"),
    ).toBeInTheDocument();
  });

  it("dispatches a per-source trigger", async () => {
    useAdminAuth.getState().setToken(ADMIN_TEST_TOKEN);
    renderAdmin();
    const button = await screen.findByTestId(
      "poll-english-manga-trusted",
      undefined,
      { timeout: 3000 },
    );
    fireEvent.click(button);
    await waitFor(() => {
      expect(
        screen.getByText(/english-manga-trusted: triggered/),
      ).toBeInTheDocument();
    });
  });
});
