import {
  Alert,
  Anchor,
  Badge,
  Button,
  Card,
  Center,
  Container,
  Group,
  Loader,
  Paper,
  SimpleGrid,
  Stack,
  Text,
  Title,
  Tooltip,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import {
  usePollAllSources,
  usePollSource,
  useRefreshAllProviders,
  useRefreshProvider,
} from "@/api/mutations";
import {
  type ProviderConfigDto,
  type ProviderDto,
  type SourceConfigDto,
  type SourceDto,
  type SourceMetricsBucket,
  type SourceMetricsSummaryItem,
  useProviderMetricsSummary,
  useProviders,
  useReviewQueueMetrics,
  useSourceMetricsDetail,
  useSourceMetricsSummary,
  useSources,
} from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";
import { AdminAuthGate } from "@/components/AdminAuthGate";
import { useAdminAuth } from "@/stores/auth";

export function AdminPage() {
  return (
    <AdminAuthGate>
      <AdminDashboard />
    </AdminAuthGate>
  );
}

function AdminDashboard() {
  const clearToken = useAdminAuth((s) => s.clear);
  return (
    <Container size="xl" py="lg">
      <Stack gap="xl">
        <Group justify="space-between" align="baseline" wrap="wrap">
          <Stack gap={2}>
            <Title order={2}>Admin</Title>
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

        <SourcesSection />
        <ProvidersSection />
        <MetricsSection />
      </Stack>
    </Container>
  );
}

function SourcesSection() {
  const sources = useSources();
  const pollAll = usePollAllSources();

  const handlePollAll = () => {
    pollAll.mutate(undefined, {
      onSuccess: (data) => {
        const triggered = data?.results.filter((r) => r.triggered).length ?? 0;
        const skipped = data?.results.filter((r) => r.skipped).length ?? 0;
        notifications.show({
          color: triggered > 0 ? "blue" : "gray",
          message: `${triggered} triggered, ${skipped} already running`,
        });
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Trigger-all failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Stack gap="md">
      <Group justify="space-between" align="baseline" wrap="wrap">
        <Stack gap={2}>
          <Title order={3}>Discovery sources</Title>
          <Text size="sm" c="dimmed">
            {sources.isLoading
              ? "loading…"
              : `${sources.data?.items.length ?? 0} configured`}
          </Text>
        </Stack>
        <Button
          size="xs"
          variant="light"
          onClick={handlePollAll}
          loading={pollAll.isPending}
          disabled={!sources.data?.items.length}
          data-testid="poll-all-sources"
        >
          Trigger all
        </Button>
      </Group>

      {sources.isError && (
        <Alert color="red" title="Failed to load sources">
          {(sources.error as Error)?.message ?? "Unknown error"}
        </Alert>
      )}

      {sources.isLoading && !sources.data && (
        <Center py="lg">
          <Loader />
        </Center>
      )}

      {sources.data && sources.data.items.length === 0 && (
        <Alert color="gray" title="No sources registered">
          Add `[[sources]]` entries in the tsundoku config and restart.
        </Alert>
      )}

      {sources.data && sources.data.items.length > 0 && (
        <SimpleGrid cols={{ base: 1, md: 2 }} spacing="md">
          {sources.data.items.map((src) => (
            <SourceCard key={src.name} source={src} />
          ))}
        </SimpleGrid>
      )}
    </Stack>
  );
}

function SourceCard({ source }: { source: SourceDto }) {
  const poll = usePollSource();
  const busy = poll.isPending;

  const handlePoll = () => {
    poll.mutate(source.name, {
      onSuccess: (data) => {
        if (data?.skipped) {
          notifications.show({
            color: "gray",
            message: `${source.name}: tick already in flight`,
          });
        } else {
          notifications.show({
            color: "blue",
            message: `${source.name}: triggered`,
          });
        }
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: `${source.name}: trigger failed`,
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Paper
      withBorder
      radius="md"
      p="md"
      data-testid={`source-card-${source.name}`}
    >
      <Stack gap="sm">
        <Group justify="space-between" align="flex-start" wrap="nowrap">
          <Stack gap={2} style={{ minWidth: 0 }}>
            <Group gap="xs" align="baseline" wrap="wrap">
              <Text fw={600}>{source.name}</Text>
              <Badge size="xs" color="indigo" variant="light">
                {source.kind}
              </Badge>
              {source.config?.enabled === false && (
                <Badge size="xs" color="gray">
                  disabled
                </Badge>
              )}
            </Group>
            <SourceStatusLine source={source} />
          </Stack>
          <Button
            size="xs"
            variant="light"
            onClick={handlePoll}
            loading={busy}
            disabled={source.config?.enabled === false}
            data-testid={`poll-${source.name}`}
          >
            Trigger
          </Button>
        </Group>
        <SourceConfigBlock config={source.config} />
      </Stack>
    </Paper>
  );
}

function SourceStatusLine({ source }: { source: SourceDto }) {
  if (source.lastError) {
    return (
      <Group gap={6} wrap="wrap">
        <Badge size="xs" color="red" variant="light">
          last error
        </Badge>
        <Text
          size="xs"
          c="red"
          title={
            typeof source.lastPolledAt === "number"
              ? formatAbsolute(source.lastPolledAt)
              : undefined
          }
        >
          {source.lastError}
        </Text>
      </Group>
    );
  }
  if (source.lastPolledAt) {
    return (
      <Group gap={6} wrap="wrap">
        <Text size="xs" c="dimmed" title={formatAbsolute(source.lastPolledAt)}>
          polled {formatRelative(source.lastPolledAt)}
        </Text>
        {source.lastSummary && (
          <Text size="xs" c="dimmed">
            • {source.lastSummary}
          </Text>
        )}
      </Group>
    );
  }
  return (
    <Text size="xs" c="dimmed">
      never polled
    </Text>
  );
}

function SourceConfigBlock({ config }: { config?: SourceConfigDto | null }) {
  if (!config) {
    return (
      <Text size="xs" c="dimmed">
        no config block bound (test scaffolding?)
      </Text>
    );
  }
  return (
    <Stack gap={4}>
      <ConfigRow label="cron" value={config.cron ?? "—"} mono />
      {config.feedUrl && (
        <ConfigRow
          label="feed_url"
          value={
            <Anchor
              href={config.feedUrl}
              size="xs"
              target="_blank"
              rel="noreferrer noopener"
              lineClamp={1}
              title={config.feedUrl}
            >
              {config.feedUrl}
            </Anchor>
          }
        />
      )}
      <ConfigRow label="timeout" value={`${config.timeoutSeconds}s`} mono />
      <ConfigRow
        label="fetch_details"
        value={config.fetchDetails ? "yes" : "no"}
        mono
      />
    </Stack>
  );
}

function ProvidersSection() {
  const providers = useProviders();
  const refreshAll = useRefreshAllProviders();

  const handleRefreshAll = () => {
    refreshAll.mutate(undefined, {
      onSuccess: (data) => {
        const triggered = data?.results.filter((r) => r.triggered).length ?? 0;
        const skipped = data?.results.filter((r) => r.skipped).length ?? 0;
        notifications.show({
          color: triggered > 0 ? "blue" : "gray",
          message: `${triggered} triggered, ${skipped} already running`,
        });
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Refresh-all failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Stack gap="md">
      <Group justify="space-between" align="baseline" wrap="wrap">
        <Stack gap={2}>
          <Title order={3}>Metadata providers</Title>
          <Text size="sm" c="dimmed">
            {providers.isLoading
              ? "loading…"
              : `${providers.data?.items.length ?? 0} configured`}
          </Text>
        </Stack>
        <Button
          size="xs"
          variant="light"
          onClick={handleRefreshAll}
          loading={refreshAll.isPending}
          disabled={!providers.data?.items.length}
          data-testid="refresh-all-providers"
        >
          Refresh all
        </Button>
      </Group>

      {providers.isError && (
        <Alert color="red" title="Failed to load providers">
          {(providers.error as Error)?.message ?? "Unknown error"}
        </Alert>
      )}

      {providers.isLoading && !providers.data && (
        <Center py="lg">
          <Loader />
        </Center>
      )}

      {providers.data && providers.data.items.length > 0 && (
        <SimpleGrid cols={{ base: 1, md: 2 }} spacing="md">
          {providers.data.items.map((p) => (
            <ProviderCard key={p.id} provider={p} />
          ))}
        </SimpleGrid>
      )}
    </Stack>
  );
}

function ProviderCard({ provider }: { provider: ProviderDto }) {
  const refresh = useRefreshProvider();

  const handleRefresh = () => {
    refresh.mutate(provider.id, {
      onSuccess: (data) => {
        if (data?.skipped) {
          notifications.show({
            color: "gray",
            message: `${provider.id}: refresh already in flight`,
          });
        } else {
          notifications.show({
            color: "blue",
            message: `${provider.id}: refresh triggered`,
          });
        }
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: `${provider.id}: refresh failed`,
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Paper
      withBorder
      radius="md"
      p="md"
      data-testid={`provider-card-${provider.id}`}
    >
      <Stack gap="sm">
        <Group justify="space-between" align="flex-start" wrap="nowrap">
          <Stack gap={2} style={{ minWidth: 0 }}>
            <Group gap="xs" align="baseline" wrap="wrap">
              <Text fw={600}>{provider.displayName}</Text>
              <Badge size="xs" color="indigo" variant="light">
                {provider.id}
              </Badge>
              {provider.active && (
                <Badge size="xs" color="teal">
                  active
                </Badge>
              )}
            </Group>
            <ProviderStatusLine provider={provider} />
          </Stack>
          <Button
            size="xs"
            variant="light"
            onClick={handleRefresh}
            loading={refresh.isPending}
            data-testid={`refresh-${provider.id}`}
          >
            Refresh
          </Button>
        </Group>
        <ProviderConfigBlock config={provider.config} />
      </Stack>
    </Paper>
  );
}

function ProviderStatusLine({ provider }: { provider: ProviderDto }) {
  const ref = provider.lastRefresh;
  if (!ref) {
    return (
      <Text size="xs" c="dimmed">
        cache never refreshed
      </Text>
    );
  }
  const parts: string[] = [];
  if (typeof ref.recordCount === "number") {
    parts.push(`${ref.recordCount.toLocaleString()} records`);
  }
  if (typeof ref.bytesDownloaded === "number") {
    parts.push(formatBytes(ref.bytesDownloaded));
  }
  return (
    <Group gap={6} wrap="wrap">
      <Text size="xs" c="dimmed" title={formatAbsolute(ref.fetchedAt)}>
        refreshed {formatRelative(ref.fetchedAt)}
      </Text>
      {parts.length > 0 && (
        <Text size="xs" c="dimmed">
          • {parts.join(", ")}
        </Text>
      )}
    </Group>
  );
}

function ProviderConfigBlock({
  config,
}: {
  config?: ProviderConfigDto | null;
}) {
  if (!config) {
    return (
      <Text size="xs" c="dimmed">
        no typed config block for this provider
      </Text>
    );
  }
  return (
    <Stack gap={4}>
      <ConfigRow
        label="api_key"
        value={
          <Badge
            size="xs"
            color={config.apiKeySet ? "teal" : "gray"}
            variant="light"
            data-testid="api-key-set-badge"
          >
            {config.apiKeySet ? "set" : "not set"}
          </Badge>
        }
      />
      <ConfigRow
        label="api_fallback"
        value={config.apiFallback ? "on" : "off"}
        mono
      />
      <ConfigRow
        label="api_base_url"
        value={
          <Text
            size="xs"
            ff="monospace"
            lineClamp={1}
            title={config.apiBaseUrl}
          >
            {config.apiBaseUrl}
          </Text>
        }
      />
      <ConfigRow
        label="offline_cache"
        value={
          <Badge
            size="xs"
            color={config.offlineCacheLoaded ? "teal" : "gray"}
            variant="light"
          >
            {config.offlineCacheLoaded ? "loaded" : "not loaded"}
          </Badge>
        }
      />
      <ConfigRow
        label="offline_dump_url"
        value={
          config.offlineDumpUrl ? (
            <Anchor
              size="xs"
              href={config.offlineDumpUrl}
              target="_blank"
              rel="noreferrer noopener"
              lineClamp={1}
              title={config.offlineDumpUrl}
            >
              {config.offlineDumpUrl}
            </Anchor>
          ) : (
            "—"
          )
        }
      />
      <ConfigRow
        label="refresh_cron"
        value={config.offlineRefreshCron ?? "—"}
        mono
      />
      <ConfigRow
        label="negative_ttl"
        value={`${config.negativeCacheTtlDays}d`}
        mono
      />
    </Stack>
  );
}

function ConfigRow({
  label,
  value,
  mono,
}: {
  label: string;
  value: React.ReactNode;
  mono?: boolean;
}) {
  return (
    <Group gap="xs" wrap="nowrap" align="baseline">
      <Text
        size="xs"
        c="dimmed"
        ff="monospace"
        miw={120}
        style={{ flexShrink: 0 }}
      >
        {label}
      </Text>
      {typeof value === "string" ? (
        <Text size="xs" ff={mono ? "monospace" : undefined}>
          {value}
        </Text>
      ) : (
        <Card padding={0} radius={0} bg="transparent" style={{ minWidth: 0 }}>
          {value}
        </Card>
      )}
    </Group>
  );
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(value >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

const METRICS_RANGE_OPTIONS = [
  { label: "1h", range: "1h" },
  { label: "24h", range: "24h" },
  { label: "7d", range: "7d" },
];

function MetricsSection() {
  const [range, setRange] = useState("24h");
  const summary = useSourceMetricsSummary({ range });
  const providers = useProviderMetricsSummary({ range });

  const sortedSources = (summary.data?.items ?? [])
    .slice()
    .sort((a, b) => a.sourceName.localeCompare(b.sourceName));

  return (
    <Stack gap="md">
      <Group justify="space-between" align="baseline" wrap="wrap">
        <Stack gap={2}>
          <Title order={3}>Metrics</Title>
          <Text size="sm" c="dimmed">
            Historical run totals over the selected window. Backed by{" "}
            <Text span ff="monospace">
              poll_runs
            </Text>{" "}
            and{" "}
            <Text span ff="monospace">
              provider_refreshes
            </Text>
            .
          </Text>
        </Stack>
        <Group gap={4} data-testid="metrics-range-picker">
          {METRICS_RANGE_OPTIONS.map((opt) => (
            <Button
              key={opt.range}
              size="xs"
              variant={range === opt.range ? "filled" : "default"}
              onClick={() => setRange(opt.range)}
            >
              {opt.label}
            </Button>
          ))}
        </Group>
      </Group>

      {summary.isError && (
        <Alert color="red" title="Failed to load metrics">
          {(summary.error as Error)?.message ?? "Unknown error"}
        </Alert>
      )}

      {summary.isLoading && !summary.data && (
        <Center py="lg">
          <Loader />
        </Center>
      )}

      {summary.data && sortedSources.length === 0 && (
        <Alert color="gray" title="No runs recorded yet">
          The first scheduler tick (or a manual{" "}
          <Text span ff="monospace">
            trigger
          </Text>
          ) populates this view.
        </Alert>
      )}

      {sortedSources.length > 0 && (
        <SimpleGrid cols={{ base: 1, md: 2 }} spacing="md">
          {sortedSources.map((item) => (
            <SourceMetricsCard
              key={item.sourceName}
              item={item}
              range={range}
            />
          ))}
        </SimpleGrid>
      )}

      {providers.data && providers.data.items.length > 0 && (
        <Stack gap="xs">
          <Title order={4}>Provider refreshes</Title>
          <SimpleGrid cols={{ base: 1, md: 2 }} spacing="md">
            {providers.data.items.map((item) => (
              <ProviderMetricsCard key={item.providerId} item={item} />
            ))}
          </SimpleGrid>
        </Stack>
      )}

      <ReviewQueueMetricsCard range={range} />
    </Stack>
  );
}

function ReviewQueueMetricsCard({ range }: { range: string }) {
  const metrics = useReviewQueueMetrics({ range });
  const snapshots = metrics.data?.snapshots ?? [];
  const latest = snapshots[snapshots.length - 1];
  const oldestSeconds = latest?.oldestPendingSeconds ?? null;
  const ttDecision = metrics.data?.timeToDecisionP50Seconds ?? null;
  return (
    <Paper
      withBorder
      radius="md"
      p="md"
      data-testid="review-queue-metrics-card"
    >
      <Stack gap="sm">
        <Group justify="space-between" align="baseline" wrap="wrap">
          <Title order={4}>Review queue</Title>
          {typeof latest?.pendingCount === "number" && (
            <Badge size="xs" variant="light" color="orange">
              {latest.pendingCount} pending
            </Badge>
          )}
        </Group>
        <Group gap="md" wrap="wrap">
          <MetricStat label="snapshots" value={snapshots.length} />
          <MetricStat label="closed" value={metrics.data?.closedCount ?? 0} />
          <Stack gap={0} miw={56}>
            <Text size="lg" fw={600} lh={1}>
              {formatDuration(oldestSeconds)}
            </Text>
            <Text size="xs" c="dimmed" tt="uppercase">
              oldest pending
            </Text>
          </Stack>
          <Stack gap={0} miw={56}>
            <Text size="lg" fw={600} lh={1}>
              {formatDuration(ttDecision)}
            </Text>
            <Text size="xs" c="dimmed" tt="uppercase">
              median time-to-decision
            </Text>
          </Stack>
        </Group>
        <DepthSparkline snapshots={snapshots} />
      </Stack>
    </Paper>
  );
}

function DepthSparkline({
  snapshots,
}: {
  snapshots: import("@/api/queries").ReviewQueueSnapshotDto[];
}) {
  if (snapshots.length === 0) {
    return (
      <Text size="xs" c="dimmed" ta="center">
        no snapshots yet — first one lands at the next hourly tick
      </Text>
    );
  }
  const width = 200;
  const height = 32;
  const slot = width / Math.max(snapshots.length, 1);
  const max = Math.max(1, ...snapshots.map((s) => s.pendingCount));
  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      data-testid="review-queue-depth-sparkline"
      aria-label="review-queue depth"
    >
      <polyline
        fill="none"
        stroke="var(--mantine-color-orange-5)"
        strokeWidth={1.5}
        points={snapshots
          .map((s, i) => {
            const x = i * slot + slot / 2;
            const y = height - (s.pendingCount / max) * height;
            return `${x.toFixed(1)},${y.toFixed(1)}`;
          })
          .join(" ")}
      />
    </svg>
  );
}

function formatDuration(seconds: number | null | undefined): string {
  if (
    typeof seconds !== "number" ||
    !Number.isFinite(seconds) ||
    seconds <= 0
  ) {
    return "—";
  }
  if (seconds < 60) return `${Math.round(seconds)}s`;
  const m = Math.round(seconds / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(seconds / 3600);
  if (h < 24) return `${h}h`;
  const d = Math.round(seconds / 86400);
  return `${d}d`;
}

function SourceMetricsCard({
  item,
  range,
}: {
  item: SourceMetricsSummaryItem;
  range: string;
}) {
  const detail = useSourceMetricsDetail(item.sourceName, {
    range,
    buckets: 24,
  });
  const rate = item.successRate;
  const rateLabel =
    typeof rate === "number" ? `${Math.round(rate * 100)}%` : "—";
  const rateColor =
    typeof rate !== "number"
      ? "gray"
      : rate >= 0.95
        ? "teal"
        : rate >= 0.75
          ? "yellow"
          : "red";
  return (
    <Paper
      withBorder
      radius="md"
      p="md"
      data-testid={`metrics-card-${item.sourceName}`}
    >
      <Stack gap="sm">
        <Group justify="space-between" align="baseline" wrap="nowrap">
          <Text fw={600}>{item.sourceName}</Text>
          <Badge size="xs" color={rateColor} variant="light">
            {rateLabel} success
          </Badge>
        </Group>
        <Group gap="md" wrap="wrap">
          <MetricStat label="runs" value={item.totalRuns} />
          <MetricStat label="success" value={item.successCount} />
          <MetricStat label="fail" value={item.failureCount} />
          <MetricStat label="skip" value={item.skippedCount} />
          <MetricStat label="fetched" value={item.fetchedSum ?? 0} />
          <MetricStat label="new" value={item.newSum ?? 0} />
        </Group>
        <Sparkline
          buckets={detail.data?.buckets ?? []}
          loading={detail.isLoading}
        />
        <ResolutionOutcomeBar outcomes={item.outcomes} />
        <Group gap="md" wrap="wrap">
          <LatencyStat
            label="fetch p50"
            value={detail.data?.fetchLatency?.p50Ms ?? null}
          />
          <LatencyStat
            label="fetch p95"
            value={detail.data?.fetchLatency?.p95Ms ?? null}
          />
          <Stack gap={0} miw={64}>
            <Text size="lg" fw={600} lh={1}>
              {formatDuration(
                detail.data?.timeToResolution?.p50Seconds ?? null,
              )}
            </Text>
            <Text size="xs" c="dimmed" tt="uppercase">
              ttr p50
            </Text>
          </Stack>
          <Stack gap={0} miw={64}>
            <Text size="lg" fw={600} lh={1}>
              {detail.data?.timeToResolution?.count ?? 0}
            </Text>
            <Text size="xs" c="dimmed" tt="uppercase">
              resolved
            </Text>
          </Stack>
        </Group>
        <ErrorKindDonut buckets={detail.data?.errorKinds ?? []} />
        {typeof item.lastStartedAt === "number" && (
          <Text size="xs" c="dimmed">
            last run {formatRelative(item.lastStartedAt)} — {item.lastStatus}
          </Text>
        )}
      </Stack>
    </Paper>
  );
}

function ResolutionOutcomeBar({
  outcomes,
}: {
  outcomes: import("@/api/queries").ResolutionOutcomeBreakdown;
}) {
  const total =
    outcomes.knownId +
    outcomes.foreignId +
    outcomes.fuzzy +
    outcomes.review +
    outcomes.failed;
  if (total === 0) {
    return (
      <Text size="xs" c="dimmed">
        no resolution events yet
      </Text>
    );
  }
  const segments = [
    { label: "known id", value: outcomes.knownId, color: "teal" },
    { label: "foreign id", value: outcomes.foreignId, color: "cyan" },
    { label: "fuzzy", value: outcomes.fuzzy, color: "blue" },
    { label: "review", value: outcomes.review, color: "orange" },
    { label: "failed", value: outcomes.failed, color: "red" },
  ];
  return (
    <Stack gap={4} data-testid="outcome-breakdown">
      <Group gap={2} style={{ height: 8, overflow: "hidden", borderRadius: 4 }}>
        {segments.map(
          (s) =>
            s.value > 0 && (
              <div
                key={s.label}
                style={{
                  flex: s.value,
                  background: `var(--mantine-color-${s.color}-5)`,
                  height: "100%",
                }}
                title={`${s.label}: ${s.value}`}
              />
            ),
        )}
      </Group>
      <Group gap="xs" wrap="wrap">
        {segments.map(
          (s) =>
            s.value > 0 && (
              <Badge key={s.label} size="xs" color={s.color} variant="light">
                {s.label} {s.value}
              </Badge>
            ),
        )}
      </Group>
    </Stack>
  );
}

function ErrorKindDonut({
  buckets,
}: {
  buckets: import("@/api/queries").ErrorKindBucket[];
}) {
  if (buckets.length === 0) return null;
  const total = buckets.reduce((s, b) => s + b.count, 0);
  if (total === 0) return null;
  return (
    <Group gap="xs" wrap="wrap" data-testid="error-kind-donut">
      <Text size="xs" c="dimmed" tt="uppercase">
        failures
      </Text>
      {buckets.map((b) => (
        <Badge key={b.kind} size="xs" color="red" variant="light">
          {b.kind} {b.count}
        </Badge>
      ))}
    </Group>
  );
}

function LatencyStat({
  label,
  value,
}: {
  label: string;
  value: number | null;
}) {
  return (
    <Stack gap={0} miw={56}>
      <Text size="lg" fw={600} lh={1}>
        {typeof value === "number" && value > 0
          ? `${Math.round(value)}ms`
          : "—"}
      </Text>
      <Text size="xs" c="dimmed" tt="uppercase">
        {label}
      </Text>
    </Stack>
  );
}

function ProviderMetricsCard({
  item,
}: {
  item: import("@/api/queries").ProviderMetricsSummaryItem;
}) {
  const rate = item.successRate;
  const rateLabel =
    typeof rate === "number" ? `${Math.round(rate * 100)}%` : "—";
  const bytes = item.bytesSum ?? 0;
  return (
    <Paper
      withBorder
      radius="md"
      p="md"
      data-testid={`provider-metrics-card-${item.providerId}`}
    >
      <Stack gap="sm">
        <Group justify="space-between" align="baseline" wrap="nowrap">
          <Text fw={600}>{item.providerId}</Text>
          <Badge size="xs" variant="light">
            {rateLabel} success
          </Badge>
        </Group>
        <Group gap="md" wrap="wrap">
          <MetricStat label="runs" value={item.totalRuns} />
          <MetricStat label="success" value={item.successCount} />
          <MetricStat label="fail" value={item.failureCount} />
          <MetricStat label="skip" value={item.skippedCount} />
        </Group>
        {bytes > 0 && (
          <Text size="xs" c="dimmed">
            transferred {formatBytes(bytes)} total
          </Text>
        )}
        {typeof item.lastStartedAt === "number" && (
          <Text size="xs" c="dimmed">
            last run {formatRelative(item.lastStartedAt)} — {item.lastStatus}
          </Text>
        )}
      </Stack>
    </Paper>
  );
}

function MetricStat({ label, value }: { label: string; value: number }) {
  return (
    <Stack gap={0} miw={56}>
      <Text size="lg" fw={600} lh={1}>
        {value.toLocaleString()}
      </Text>
      <Text size="xs" c="dimmed" tt="uppercase">
        {label}
      </Text>
    </Stack>
  );
}

// Tiny inline SVG sparkline. Each bucket renders three stacked bars
// (success, failure, skipped). Zero-value buckets render as an empty slot
// so the chart's spacing reflects the actual time window. No new chart
// dependency — sparkline is intentionally lightweight.
function Sparkline({
  buckets,
  loading,
}: {
  buckets: SourceMetricsBucket[];
  loading: boolean;
}) {
  if (loading && buckets.length === 0) {
    return (
      <Center py={6}>
        <Loader size="xs" />
      </Center>
    );
  }
  if (buckets.length === 0) {
    return (
      <Text size="xs" c="dimmed" ta="center">
        no runs in window
      </Text>
    );
  }
  const width = 200;
  const height = 32;
  const slot = width / buckets.length;
  const barWidth = Math.max(2, slot - 2);
  const maxTotal = Math.max(
    1,
    ...buckets.map((b) => b.successCount + b.failureCount + b.skippedCount),
  );

  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      aria-label="runs over time"
      data-testid="metrics-sparkline"
    >
      {buckets.map((b, i) => {
        const x = i * slot + 1;
        const successH = (b.successCount / maxTotal) * height;
        const failH = (b.failureCount / maxTotal) * height;
        const skipH = (b.skippedCount / maxTotal) * height;
        return (
          <g key={b.bucketStart}>
            {b.successCount > 0 && (
              <rect
                x={x}
                y={height - successH}
                width={barWidth}
                height={successH}
                fill="var(--mantine-color-teal-5)"
              />
            )}
            {b.failureCount > 0 && (
              <rect
                x={x}
                y={height - successH - failH}
                width={barWidth}
                height={failH}
                fill="var(--mantine-color-red-5)"
              />
            )}
            {b.skippedCount > 0 && (
              <rect
                x={x}
                y={height - successH - failH - skipH}
                width={barWidth}
                height={skipH}
                fill="var(--mantine-color-gray-4)"
              />
            )}
          </g>
        );
      })}
    </svg>
  );
}
