import {
  Alert,
  Anchor,
  Badge,
  Button,
  Group,
  Paper,
  SegmentedControl,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { Link, useParams } from "@tanstack/react-router";
import { useState } from "react";
import {
  useSourceMetricsDetail,
  useSourceRuns,
  useSources,
} from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";
import { SourceMetricsCard } from "@/components/admin/MetricsCards";
import {
  SourceConfigBlock,
  SourceStatusLine,
} from "@/components/admin/SourceCard";

const RANGE_OPTIONS = [
  { label: "1h", value: "1h" },
  { label: "24h", value: "24h" },
  { label: "7d", value: "7d" },
  { label: "30d", value: "30d" },
];

/// Per-source detail. Pulls the source row out of `useSources` (already
/// cached from the list page) and the metrics from
/// `useSourceMetricsDetail`. Range selector is local state; bookmarked
/// URLs always start at 24h.
export function AdminSourceDetailPage() {
  const { name } = useParams({ from: "/admin/sources/$name" });
  const [range, setRange] = useState("24h");
  const sources = useSources();
  const item = (sources.data?.items ?? []).find((s) => s.name === name);
  const summary = useSourceMetricsDetail(name, { range, buckets: 24 });

  if (sources.isLoading && !sources.data) {
    return (
      <Text size="sm" c="dimmed">
        loading source…
      </Text>
    );
  }

  if (!item) {
    return (
      <Alert color="gray" title={`Unknown source: ${name}`}>
        No source registered under that name.{" "}
        <Anchor component={Link} to="/admin/sources" size="sm">
          Back to sources
        </Anchor>
      </Alert>
    );
  }

  return (
    <Stack gap="md">
      <Group justify="space-between" align="baseline" wrap="wrap">
        <Stack gap={2}>
          <Anchor component={Link} to="/admin/sources" size="xs" c="dimmed">
            ← Sources
          </Anchor>
          <Title order={3}>{item.name}</Title>
          <SourceStatusLine source={item} />
        </Stack>
        <SegmentedControl
          size="xs"
          data={RANGE_OPTIONS}
          value={range}
          onChange={setRange}
          data-testid="source-detail-range"
        />
      </Group>

      <Paper withBorder radius="md" p="md">
        <Stack gap="sm">
          <Title order={5}>Config</Title>
          <SourceConfigBlock config={item.config} />
        </Stack>
      </Paper>

      {summary.data?.summary ? (
        <SourceMetricsCard item={summary.data.summary} range={range} />
      ) : (
        <Alert color="gray" title="No runs in this range">
          Nothing has run yet for this source in the selected window.
        </Alert>
      )}

      <SourceRecentRuns name={item.name} />

      <Group justify="flex-end">
        <Button component={Link} to="/admin/metrics" variant="subtle" size="xs">
          See cross-cutting metrics →
        </Button>
      </Group>
    </Stack>
  );
}

const RUN_STATUS_META: Record<string, { color: string; label: string }> = {
  success: { color: "green", label: "success" },
  failure: { color: "red", label: "failed" },
  running: { color: "blue", label: "running…" },
  skipped: { color: "gray", label: "skipped" },
};

/// Codex-"Recent syncs"-style per-run timeline: the individual runs
/// behind the aggregated metrics card. Polls, backfills, and re-enrich
/// runs share the lane; `trigger` tells them apart. Renders nothing
/// until the source has at least one recorded run.
function SourceRecentRuns({ name }: { name: string }) {
  const runs = useSourceRuns(name);
  const items = runs.data?.items ?? [];
  if (items.length === 0) return null;

  return (
    <Paper withBorder radius="md" p="md" data-testid="source-recent-runs">
      <Stack gap="sm">
        <Text size="xs" fw={600} c="dimmed" tt="uppercase">
          Recent runs
        </Text>
        <Stack gap="sm">
          {items.map((r) => {
            const meta = RUN_STATUS_META[r.status] ?? RUN_STATUS_META.failure;
            const totalMs =
              (r.fetchDurationMs ?? 0) +
              (r.enrichDurationMs ?? 0) +
              (r.resolveDurationMs ?? 0);
            return (
              <Stack key={r.id} gap={2} data-testid={`source-run-${r.id}`}>
                <Group gap="xs" wrap="nowrap" align="center">
                  <Badge
                    size="xs"
                    variant="light"
                    color={meta.color}
                    style={{ flexShrink: 0 }}
                  >
                    {meta.label}
                  </Badge>
                  <Text size="xs" c="dimmed" style={{ flexShrink: 0 }}>
                    via {r.trigger}
                  </Text>
                  <Text
                    size="xs"
                    c="dimmed"
                    style={{ flex: 1, minWidth: 0, textAlign: "right" }}
                    title={formatAbsolute(r.startedAt)}
                  >
                    {formatRelative(r.startedAt)}
                  </Text>
                </Group>
                <Group gap="xs" wrap="wrap" align="baseline" pl={4}>
                  {r.status === "success" && (
                    <Text size="xs" c="dimmed">
                      {r.fetchedCount ?? 0} fetched · {r.newCount ?? 0} new ·{" "}
                      {r.resolvedCount ?? 0} resolved
                      {totalMs > 0 && ` · ${(totalMs / 1000).toFixed(1)}s`}
                    </Text>
                  )}
                  {r.status === "failure" && (
                    <Text size="xs" c="red" lineClamp={2}>
                      {r.errorMessage ?? r.errorKind ?? "unknown error"}
                    </Text>
                  )}
                </Group>
              </Stack>
            );
          })}
        </Stack>
      </Stack>
    </Paper>
  );
}
