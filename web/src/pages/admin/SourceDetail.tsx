import {
  Alert,
  Anchor,
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
import { useSourceMetricsDetail, useSources } from "@/api/queries";
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

      <Group justify="flex-end">
        <Button component={Link} to="/admin/metrics" variant="subtle" size="xs">
          See cross-cutting metrics →
        </Button>
      </Group>
    </Stack>
  );
}
