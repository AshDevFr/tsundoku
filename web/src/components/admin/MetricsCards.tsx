import { Badge, Group, Paper, Stack, Text, Title } from "@mantine/core";
import {
  type ProviderMetricsSummaryItem,
  type SourceMetricsSummaryItem,
  useReviewQueueMetrics,
  useSourceMetricsDetail,
} from "@/api/queries";
import { formatRelative } from "@/api/utils";
import { LatencyStat, MetricStat, SuccessRateBadge } from "./atoms";
import {
  DepthSparkline,
  ErrorKindList,
  ResolutionOutcomeBar,
  Sparkline,
} from "./charts";
import { formatBytes, formatDuration } from "./format";

/// Single-source summary card used on the sources list and the
/// cross-cutting metrics page. Renders the full surface the
/// `SourceMetricsDetail` endpoint produces: sparkline, outcomes bar,
/// fetch p50/p95/**max** latency, time-to-resolution p50/p95, and the
/// failure-kind pills.
export function SourceMetricsCard({
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
  // Per-release averages so a single backfill (one row with thousands of
  // releases) sits on the same scale as a steady-state poll (many rows of
  // ~75 releases). `null` when nothing's been resolved yet — the
  // sums-over-zero division would just produce Infinity otherwise.
  const newSum = item.newSum ?? 0;
  const avgEnrichMs =
    newSum > 0 ? (item.enrichDurationMsSum ?? 0) / newSum : null;
  const avgResolveMs =
    newSum > 0 ? (item.resolveDurationMsSum ?? 0) / newSum : null;
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
          <SuccessRateBadge rate={item.successRate} />
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
            value={detail.data?.fetchLatency?.p50Ms}
          />
          <LatencyStat
            label="fetch p95"
            value={detail.data?.fetchLatency?.p95Ms}
          />
          <LatencyStat
            label="fetch max"
            value={detail.data?.fetchLatency?.maxMs}
          />
          <LatencyStat label="enrich avg" value={avgEnrichMs} />
          <LatencyStat label="resolve avg" value={avgResolveMs} />
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
              {formatDuration(
                detail.data?.timeToResolution?.p95Seconds ?? null,
              )}
            </Text>
            <Text size="xs" c="dimmed" tt="uppercase">
              ttr p95
            </Text>
          </Stack>
          <MetricStat
            label="resolved"
            value={detail.data?.timeToResolution?.count ?? 0}
          />
        </Group>
        <ErrorKindList buckets={detail.data?.errorKinds ?? []} />
        {typeof item.lastStartedAt === "number" && (
          <Text size="xs" c="dimmed">
            last run {formatRelative(item.lastStartedAt)} — {item.lastStatus}
          </Text>
        )}
      </Stack>
    </Paper>
  );
}

/// Compact provider-refresh card. The detail page renders the heavier
/// view (per-bucket history, fetch latency chart).
export function ProviderMetricsCard({
  item,
}: {
  item: ProviderMetricsSummaryItem;
}) {
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
          <SuccessRateBadge rate={item.successRate} />
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

/// Review-queue depth + median time-to-decision over the selected range.
/// The depth sparkline is the most useful signal here — it answers
/// "is the backlog growing?" at a glance.
export function ReviewQueueMetricsCard({ range }: { range: string }) {
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
