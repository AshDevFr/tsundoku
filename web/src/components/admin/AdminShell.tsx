import {
  Anchor,
  Badge,
  Box,
  Button,
  Container,
  Group,
  NavLink,
  Stack,
  Text,
  Title,
  Tooltip,
} from "@mantine/core";
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
  { label: "Sources", to: "/admin/sources", matchPrefix: "/admin/sources" },
  {
    label: "Providers",
    to: "/admin/providers",
    matchPrefix: "/admin/providers",
  },
  { label: "Metrics", to: "/admin/metrics" },
  { label: "ID Maps", to: "/admin/id-maps" },
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
  return (
    <Container size="xl" py="lg">
      <Stack gap="lg">
        <Group justify="space-between" align="baseline" wrap="wrap">
          <Stack gap={2}>
            <Group gap="sm" align="baseline">
              <Title order={2}>Admin</Title>
              <FailurePip />
            </Group>
            <Text size="sm" c="dimmed">
              Inspect runtime state and force-trigger scheduler work.
            </Text>
          </Stack>
          <Group gap="sm">
            <Anchor component={Link} to="/review" size="sm">
              Review queue →
            </Anchor>
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
        </Group>

        <Group align="flex-start" wrap="nowrap" gap="lg">
          <Box
            component="nav"
            aria-label="Admin sections"
            miw={180}
            visibleFrom="sm"
          >
            <Stack gap={2}>
              {NAV.map((entry) => (
                <NavLink
                  key={entry.to}
                  label={entry.label}
                  component={Link}
                  to={entry.to}
                  active={isActive(entry, location.pathname)}
                  data-testid={`admin-nav-${slug(entry.label)}`}
                />
              ))}
            </Stack>
          </Box>
          <MobileNav location={location.pathname} />
          <Box flex={1} miw={0}>
            <Outlet />
          </Box>
        </Group>
      </Stack>
    </Container>
  );
}

function MobileNav({ location }: { location: string }) {
  return (
    <Box hiddenFrom="sm" w="100%">
      <Group gap={4} wrap="wrap">
        {NAV.map((entry) => (
          <Button
            key={entry.to}
            component={Link}
            to={entry.to}
            size="xs"
            variant={isActive(entry, location) ? "filled" : "default"}
            data-testid={`admin-nav-mobile-${slug(entry.label)}`}
          >
            {entry.label}
          </Button>
        ))}
      </Group>
    </Box>
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
