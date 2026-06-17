import { Anchor, Badge, Group, Stack, Text } from "@mantine/core";
import type { ReactNode } from "react";

/// Single-line label/value row used across the source + provider config
/// blocks. `mono` toggles the monospace font for the value cell only.
export function ConfigRow({
  label,
  value,
  mono,
}: {
  label: string;
  value: ReactNode;
  mono?: boolean;
}) {
  return (
    <Group gap="xs" wrap="nowrap" align="baseline">
      <Text
        size="xs"
        c="dimmed"
        ff="monospace"
        w={150}
        style={{ flexShrink: 0, whiteSpace: "nowrap" }}
      >
        {label}
      </Text>
      {typeof value === "string" ? (
        // minWidth:0 lets the value shrink inside the nowrap row, and
        // overflowWrap breaks long unbreakable strings (e.g. a base_url) so
        // they wrap instead of pushing the page wider than a phone.
        <Text
          size="xs"
          ff={mono ? "monospace" : undefined}
          style={{ minWidth: 0, overflowWrap: "anywhere" }}
        >
          {value}
        </Text>
      ) : (
        value
      )}
    </Group>
  );
}

/// Stacked "big number + small uppercase label" stat used in metrics
/// cards. Numeric inputs are localized; anything else (e.g. "—") is
/// passed through as a string.
export function MetricStat({
  label,
  value,
}: {
  label: string;
  value: number | string;
}) {
  const display =
    typeof value === "number" ? value.toLocaleString() : (value ?? "—");
  return (
    <Stack gap={0} miw={56}>
      <Text size="lg" fw={600} lh={1}>
        {display}
      </Text>
      <Text size="xs" c="dimmed" tt="uppercase">
        {label}
      </Text>
    </Stack>
  );
}

/// Latency-specific stat: rounds milliseconds, hides zero/null values
/// behind an em-dash, and reuses the same MetricStat layout.
export function LatencyStat({
  label,
  value,
}: {
  label: string;
  value: number | null | undefined;
}) {
  const display =
    typeof value === "number" && value > 0 ? `${Math.round(value)}ms` : "—";
  return (
    <Stack gap={0} miw={56}>
      <Text size="lg" fw={600} lh={1}>
        {display}
      </Text>
      <Text size="xs" c="dimmed" tt="uppercase">
        {label}
      </Text>
    </Stack>
  );
}

/// Compact "x.y% success" pill that color-codes against three thresholds
/// (≥95 = teal, ≥75 = yellow, otherwise red). `null` rate renders gray.
export function SuccessRateBadge({
  rate,
}: {
  rate: number | null | undefined;
}) {
  const label = typeof rate === "number" ? `${Math.round(rate * 100)}%` : "—";
  const color =
    typeof rate !== "number"
      ? "gray"
      : rate >= 0.95
        ? "teal"
        : rate >= 0.75
          ? "yellow"
          : "red";
  return (
    <Badge size="xs" color={color} variant="light">
      {label} success
    </Badge>
  );
}

/// Inline-anchor variant used for URL fields where the visible text
/// should match the href. Truncates with an ellipsis if needed.
export function ExternalLink({ url }: { url: string }) {
  return (
    <Anchor
      href={url}
      size="xs"
      target="_blank"
      rel="noreferrer noopener"
      lineClamp={1}
      title={url}
    >
      {url}
    </Anchor>
  );
}
