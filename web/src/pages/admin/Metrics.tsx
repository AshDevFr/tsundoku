import {
  Alert,
  Center,
  Group,
  Loader,
  SegmentedControl,
  SimpleGrid,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { useState } from "react";
import {
  useProviderMetricsSummary,
  useSourceMetricsSummary,
} from "@/api/queries";
import {
  ProviderMetricsCard,
  ReviewQueueMetricsCard,
  SourceMetricsCard,
} from "@/components/admin/MetricsCards";

const RANGE_OPTIONS = [
  { label: "1h", value: "1h" },
  { label: "24h", value: "24h" },
  { label: "7d", value: "7d" },
  { label: "30d", value: "30d" },
];

/// Cross-cutting metrics page: per-source cards, per-provider cards,
/// review-queue depth chart, all over the same selected range. Range
/// is local state; if we ever want this to be linkable we can lift
/// into the URL search.
export function AdminMetricsPage() {
  const [range, setRange] = useState("24h");
  const sources = useSourceMetricsSummary({ range });
  const providers = useProviderMetricsSummary({ range });

  const sortedSources = (sources.data?.items ?? [])
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
        <SegmentedControl
          size="xs"
          data={RANGE_OPTIONS}
          value={range}
          onChange={setRange}
          data-testid="metrics-range-picker"
        />
      </Group>

      {sources.isError && (
        <Alert color="red" title="Failed to load metrics">
          {(sources.error as Error)?.message ?? "Unknown error"}
        </Alert>
      )}

      {sources.isLoading && !sources.data && (
        <Center py="lg">
          <Loader />
        </Center>
      )}

      {sources.data && sortedSources.length === 0 && (
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
          <Title order={4}>Provider cache refreshes</Title>
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
