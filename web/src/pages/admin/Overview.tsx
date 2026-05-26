import {
  Alert,
  Anchor,
  Badge,
  Card,
  Group,
  SimpleGrid,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { Link } from "@tanstack/react-router";
import {
  useIdMapMetrics,
  useProviders,
  useReviewQueueMetrics,
  useSources,
  useStats,
} from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";

/// Landing page for the admin area. Three things matter at a glance:
///
/// 1. Is anything failing right now?
/// 2. What was the last thing the system did?
/// 3. Quick stat cards that each link into their detail page.
export function AdminOverviewPage() {
  return (
    <Stack gap="lg">
      <FailureStrip />
      <QuickStats />
      <RecentActivity />
    </Stack>
  );
}

function FailureStrip() {
  const sources = useSources();
  const failing = (sources.data?.items ?? []).filter((s) => s.lastError);
  if (sources.isLoading) return null;
  if (failing.length === 0) {
    return (
      <Alert
        color="teal"
        variant="light"
        title="All sources are happy"
        data-testid="overview-all-green"
      >
        No source has reported a failure on its most recent poll.
      </Alert>
    );
  }
  return (
    <Alert
      color="red"
      variant="light"
      title={`${failing.length} source(s) reporting an error`}
      data-testid="overview-failure-strip"
    >
      <Stack gap="xs">
        {failing.map((s) => (
          <Group gap="xs" key={s.name} wrap="wrap">
            <Link
              to="/admin/sources/$name"
              params={{ name: s.name }}
              style={{ textDecoration: "none", color: "inherit" }}
            >
              <Text size="sm" fw={600} component="span">
                {s.name}
              </Text>
            </Link>
            <Text size="xs" c="red">
              {s.lastError}
            </Text>
            {typeof s.lastPolledAt === "number" && (
              <Text size="xs" c="dimmed">
                ({formatRelative(s.lastPolledAt)})
              </Text>
            )}
          </Group>
        ))}
      </Stack>
    </Alert>
  );
}

function QuickStats() {
  const stats = useStats();
  const review = useReviewQueueMetrics({ range: "24h" });
  const idMaps = useIdMapMetrics();
  const reviewLatest = review.data?.snapshots[review.data.snapshots.length - 1];
  const muTotal =
    (idMaps.data?.mangaupdatesRedirectCache.modernCount ?? 0) +
    (idMaps.data?.mangaupdatesRedirectCache.tombstoneCount ?? 0);
  const externalTotal = (idMaps.data?.externalIds ?? []).reduce(
    (s, e) => s + e.count,
    0,
  );

  return (
    <SimpleGrid cols={{ base: 1, sm: 3 }} spacing="md">
      <StatCard
        title="Catalog"
        value={stats.data?.series ?? 0}
        unit="series"
        href="/"
      />
      <StatCard
        title="Review queue"
        value={reviewLatest?.pendingCount ?? 0}
        unit="pending"
        href="/admin/metrics"
        accent={(reviewLatest?.pendingCount ?? 0) > 0 ? "orange" : undefined}
      />
      <StatCard
        title="ID maps"
        value={externalTotal + muTotal}
        unit="mappings"
        href="/admin/id-maps"
      />
    </SimpleGrid>
  );
}

function StatCard({
  title,
  value,
  unit,
  href,
  accent,
}: {
  title: string;
  value: number;
  unit: string;
  href: string;
  accent?: string;
}) {
  return (
    <Card
      component={Link}
      to={href}
      withBorder
      radius="md"
      p="md"
      data-testid={`overview-stat-${title.toLowerCase().replace(/\s+/g, "-")}`}
      style={{ textDecoration: "none", color: "inherit" }}
    >
      <Stack gap={4}>
        <Text size="xs" c="dimmed" tt="uppercase">
          {title}
        </Text>
        <Group gap="xs" align="baseline">
          <Text fw={700} size="xl" lh={1} c={accent}>
            {value.toLocaleString()}
          </Text>
          <Text size="sm" c="dimmed">
            {unit}
          </Text>
        </Group>
      </Stack>
    </Card>
  );
}

function RecentActivity() {
  const sources = useSources();
  const providers = useProviders();
  const entries = [
    ...(sources.data?.items ?? []).flatMap((s) =>
      typeof s.lastPolledAt === "number"
        ? [
            {
              kind: "source" as const,
              id: s.name,
              ts: s.lastPolledAt,
              line: s.lastError
                ? `error: ${s.lastError}`
                : (s.lastSummary ?? "polled"),
              failed: Boolean(s.lastError),
            },
          ]
        : [],
    ),
    ...(providers.data?.items ?? []).flatMap((p) =>
      p.lastRefresh
        ? [
            {
              kind: "provider" as const,
              id: p.id,
              ts: p.lastRefresh.fetchedAt,
              line:
                typeof p.lastRefresh.recordCount === "number"
                  ? `cache refreshed (${p.lastRefresh.recordCount.toLocaleString()} records)`
                  : "cache refreshed",
              failed: false,
            },
          ]
        : [],
    ),
  ].sort((a, b) => b.ts - a.ts);

  if (entries.length === 0) {
    return (
      <Stack gap="xs">
        <Title order={3}>Recent activity</Title>
        <Text size="sm" c="dimmed">
          Nothing has run yet. Trigger a poll from{" "}
          <Anchor component={Link} to="/admin/sources" size="sm">
            Sources
          </Anchor>
          .
        </Text>
      </Stack>
    );
  }

  return (
    <Stack gap="xs">
      <Title order={3}>Recent activity</Title>
      <Stack gap={4}>
        {entries.slice(0, 8).map((e) => (
          <Group key={`${e.kind}-${e.id}-${e.ts}`} gap="xs" wrap="wrap">
            <Badge
              size="xs"
              variant="light"
              color={e.kind === "source" ? "indigo" : "grape"}
            >
              {e.kind}
            </Badge>
            {e.kind === "source" ? (
              <Link
                to="/admin/sources/$name"
                params={{ name: e.id }}
                style={{ textDecoration: "none", color: "inherit" }}
              >
                <Text size="sm" fw={600} component="span">
                  {e.id}
                </Text>
              </Link>
            ) : (
              <Link
                to="/admin/providers/$id"
                params={{ id: e.id }}
                style={{ textDecoration: "none", color: "inherit" }}
              >
                <Text size="sm" fw={600} component="span">
                  {e.id}
                </Text>
              </Link>
            )}
            <Text size="xs" c={e.failed ? "red" : "dimmed"}>
              {e.line}
            </Text>
            <Text size="xs" c="dimmed" title={formatAbsolute(e.ts)}>
              ({formatRelative(e.ts)})
            </Text>
          </Group>
        ))}
      </Stack>
    </Stack>
  );
}
