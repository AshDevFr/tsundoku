import { Badge, Center, Group, Loader, Stack, Text } from "@mantine/core";
import type {
  ErrorKindBucket,
  ProviderMetricsBucket,
  ResolutionOutcomeBreakdown,
  ReviewQueueSnapshotDto,
  SourceMetricsBucket,
} from "@/api/queries";

/// Stacked SVG bars: success (teal) on the bottom, failure (red), then
/// skipped (gray). Inline so the bundle does not pull in a chart lib for
/// a 32px tall widget that ships in every admin page.
export function Sparkline({
  buckets,
  loading,
}: {
  buckets: SourceMetricsBucket[] | ProviderMetricsBucket[];
  loading?: boolean;
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

/// Continuous depth line for the review queue snapshots.
export function DepthSparkline({
  snapshots,
}: {
  snapshots: ReviewQueueSnapshotDto[];
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

/// Five-segment stacked bar + legend pills for the resolution
/// outcome breakdown over a window.
export function ResolutionOutcomeBar({
  outcomes,
}: {
  outcomes: ResolutionOutcomeBreakdown;
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

/// One pill per failure kind. Hidden entirely when there are no
/// failures in the window — avoids a useless empty row in the card.
export function ErrorKindList({ buckets }: { buckets: ErrorKindBucket[] }) {
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
