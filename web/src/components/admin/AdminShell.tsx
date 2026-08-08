import {
  Badge,
  Box,
  Burger,
  Button,
  Container,
  Drawer,
  Group,
  NavLink,
  Stack,
  Text,
  Title,
  Tooltip,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { Link, Outlet, useLocation } from "@tanstack/react-router";
import { JobEventsProvider } from "@/api/jobEventsContext";
import { type SourceDto, useSources } from "@/api/queries";
import { AdminAuthGate } from "@/components/AdminAuthGate";
import { useAdminAuth } from "@/stores/auth";

interface NavEntry {
  label: string;
  to: string;
  /** Match this exact path AND any deeper subpath. */
  matchPrefix?: string;
}

const NAV: NavEntry[] = [
  { label: "Overview", to: "/admin" },
  { label: "Review", to: "/admin/review" },
  { label: "Wishlist", to: "/admin/wishlist" },
  { label: "Kept", to: "/admin/kept" },
  { label: "Releases", to: "/admin/releases" },
  { label: "Sources", to: "/admin/sources", matchPrefix: "/admin/sources" },
  {
    label: "Providers",
    to: "/admin/providers",
    matchPrefix: "/admin/providers",
  },
  { label: "Download", to: "/admin/download" },
  { label: "Codex", to: "/admin/codex" },
  { label: "Metrics", to: "/admin/metrics" },
  { label: "ID Maps", to: "/admin/id-maps" },
  { label: "Maintenance", to: "/admin/maintenance" },
  { label: "Export", to: "/admin/export" },
];

/// Layout wrapper for every page under `/admin/*`. Hosts the auth gate
/// (kept mounted across child navigations so we don't re-prompt for the
/// token), the sticky nav rail, and a header that surfaces failure
/// state and a sign-out button.
export function AdminShell() {
  return (
    <AdminAuthGate>
      <JobEventsProvider>
        <AdminLayout />
      </JobEventsProvider>
    </AdminAuthGate>
  );
}

function AdminLayout() {
  const clearToken = useAdminAuth((s) => s.clear);
  const location = useLocation();
  // The section nav is a left drawer on mobile (freeing the content column to
  // full width); the desktop rail is always visible and never opens this.
  const [navOpened, { toggle: toggleNav, close: closeNav }] =
    useDisclosure(false);
  return (
    <Container size="xl" py="lg">
      <Stack gap="lg">
        <Group justify="space-between" align="center" wrap="nowrap">
          <Group gap="sm" align="center" wrap="nowrap">
            <Burger
              opened={navOpened}
              onClick={toggleNav}
              size="sm"
              hiddenFrom="sm"
              aria-label={
                navOpened ? "Close admin sections" : "Open admin sections"
              }
            />
            <Stack gap={2}>
              <Group gap="sm" align="baseline">
                <Title order={2}>Admin</Title>
                <FailurePip />
              </Group>
              <Text size="sm" c="dimmed" visibleFrom="sm">
                Inspect runtime state and force-trigger scheduler work.
              </Text>
            </Stack>
          </Group>
          <Tooltip label="Forget the admin token in this browser">
            <Button
              variant="subtle"
              size="xs"
              color="gray"
              onClick={() => clearToken()}
            >
              Sign out
            </Button>
          </Tooltip>
        </Group>

        <Group align="flex-start" wrap="nowrap" gap="lg">
          <Box
            component="nav"
            aria-label="Admin sections"
            miw={180}
            visibleFrom="sm"
          >
            <AdminNavLinks
              location={location.pathname}
              testIdPrefix="admin-nav-"
            />
          </Box>
          <Box flex={1} miw={0}>
            <Outlet />
          </Box>
        </Group>
      </Stack>

      <Drawer
        opened={navOpened}
        onClose={closeNav}
        position="left"
        size="xs"
        title="Admin sections"
        hiddenFrom="sm"
      >
        <AdminNavLinks
          location={location.pathname}
          testIdPrefix="admin-nav-mobile-"
          onNavigate={closeNav}
        />
      </Drawer>
    </Container>
  );
}

/// The admin section links, shared by the always-on desktop rail and the
/// mobile drawer. The two render the same entries with distinct testid
/// prefixes so each stays individually queryable.
function AdminNavLinks({
  location,
  testIdPrefix,
  onNavigate,
}: {
  location: string;
  testIdPrefix: string;
  onNavigate?: () => void;
}) {
  return (
    <Stack gap={2}>
      {NAV.map((entry) => (
        <NavLink
          key={entry.to}
          label={entry.label}
          component={Link}
          to={entry.to}
          activeOptions={{ exact: true }}
          active={isActive(entry, location)}
          onClick={onNavigate}
          data-testid={`${testIdPrefix}${slug(entry.label)}`}
        />
      ))}
    </Stack>
  );
}

function isActive(entry: NavEntry, pathname: string): boolean {
  if (entry.matchPrefix) {
    return (
      pathname === entry.matchPrefix ||
      pathname.startsWith(`${entry.matchPrefix}/`)
    );
  }
  return pathname === entry.to;
}

function slug(s: string): string {
  return s.toLowerCase().replace(/\s+/g, "-");
}

/// Red "something is failing" dot next to the page title. Driven off
/// the same data the Overview tab uses, so the two surfaces stay in
/// sync. The pip stays hidden when every source and provider is happy.
function FailurePip() {
  const sources = useSources();
  const count = countSourceFailures(sources.data?.items);
  if (count === 0) return null;
  return (
    <Tooltip label={`${count} source(s) reporting an error`}>
      <Badge
        size="sm"
        color="red"
        variant="filled"
        data-testid="admin-failure-pip"
      >
        {count} failing
      </Badge>
    </Tooltip>
  );
}

/// Count sources whose last poll surfaced an error. Provider refresh
/// failures don't have an equivalent `lastError` field on
/// `ProviderDto`; the metrics page is where you go to see them.
export function countSourceFailures(sources: SourceDto[] | undefined): number {
  let n = 0;
  for (const s of sources ?? []) {
    if (s.lastError) n++;
  }
  return n;
}
