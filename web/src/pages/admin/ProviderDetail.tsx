import {
  Alert,
  Anchor,
  Group,
  Paper,
  SegmentedControl,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { Link, useParams } from "@tanstack/react-router";
import { useState } from "react";
import { useProviderMetricsDetail, useProviders } from "@/api/queries";
import {
  LatencyStat,
  MetricStat,
  SuccessRateBadge,
} from "@/components/admin/atoms";
import { Sparkline } from "@/components/admin/charts";
import { formatBytes } from "@/components/admin/format";
import {
  ProviderConfigBlock,
  ProviderStatusLine,
} from "@/components/admin/ProviderCard";

const RANGE_OPTIONS = [
  { label: "24h", value: "24h" },
  { label: "7d", value: "7d" },
  { label: "30d", value: "30d" },
];

/// Per-provider detail. Larger surface than the list card: full
/// `ProviderMetricsDetail` with the refresh-history bar chart and
/// fetch-latency p50/p95/max.
export function AdminProviderDetailPage() {
  const { id } = useParams({ from: "/admin/providers/$id" });
  const [range, setRange] = useState("7d");
  const providers = useProviders();
  const item = (providers.data?.items ?? []).find((p) => p.id === id);
  const detail = useProviderMetricsDetail(id, { range, buckets: 24 });

  if (providers.isLoading && !providers.data) {
    return (
      <Text size="sm" c="dimmed">
        loading provider…
      </Text>
    );
  }

  if (!item) {
    return (
      <Alert color="gray" title={`Unknown provider: ${id}`}>
        No provider registered under that id.{" "}
        <Anchor component={Link} to="/admin/providers" size="sm">
          Back to providers
        </Anchor>
      </Alert>
    );
  }

  return (
    <Stack gap="md">
      <Group justify="space-between" align="baseline" wrap="wrap">
        <Stack gap={2}>
          <Anchor component={Link} to="/admin/providers" size="xs" c="dimmed">
            ← Providers
          </Anchor>
          <Title order={3}>{item.displayName}</Title>
          <Text size="xs" c="dimmed" ff="monospace">
            {item.id}
          </Text>
          <ProviderStatusLine provider={item} />
        </Stack>
        <SegmentedControl
          size="xs"
          data={RANGE_OPTIONS}
          value={range}
          onChange={setRange}
          data-testid="provider-detail-range"
        />
      </Group>

      <Paper withBorder radius="md" p="md">
        <Stack gap="sm">
          <Title order={5}>Config</Title>
          <ProviderConfigBlock config={item.config} />
        </Stack>
      </Paper>

      {detail.data?.summary ? (
        <Paper withBorder radius="md" p="md">
          <Stack gap="sm">
            <Group justify="space-between" align="baseline" wrap="nowrap">
              <Title order={5}>Refresh history</Title>
              <SuccessRateBadge rate={detail.data.summary.successRate} />
            </Group>
            <Group gap="md" wrap="wrap">
              <MetricStat label="runs" value={detail.data.summary.totalRuns} />
              <MetricStat
                label="success"
                value={detail.data.summary.successCount}
              />
              <MetricStat
                label="fail"
                value={detail.data.summary.failureCount}
              />
              <MetricStat
                label="skip"
                value={detail.data.summary.skippedCount}
              />
              <MetricStat
                label="downloaded"
                value={formatBytes(detail.data.summary.bytesSum ?? 0)}
              />
            </Group>
            <Sparkline
              buckets={detail.data.buckets}
              loading={detail.isLoading}
            />
            <Group gap="md" wrap="wrap">
              <LatencyStat
                label="fetch p50"
                value={detail.data.fetchLatency?.p50Ms}
              />
              <LatencyStat
                label="fetch p95"
                value={detail.data.fetchLatency?.p95Ms}
              />
              <LatencyStat
                label="fetch max"
                value={detail.data.fetchLatency?.maxMs}
              />
            </Group>
          </Stack>
        </Paper>
      ) : (
        <Alert color="gray" title="No refreshes in this range">
          The provider hasn't refreshed in the selected window.
        </Alert>
      )}
    </Stack>
  );
}
