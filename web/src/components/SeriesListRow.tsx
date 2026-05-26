import {
  AspectRatio,
  Badge,
  Box,
  Group,
  Image,
  Paper,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { Link } from "@tanstack/react-router";
import type { SeriesListItem } from "@/api/queries";
import { formatRelative } from "@/api/utils";

const COVER_PLACEHOLDER =
  "data:image/svg+xml;utf8,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 3 4%22%3E%3Crect width=%223%22 height=%224%22 fill=%22%23ced4da%22/%3E%3C/svg%3E";

/// Horizontal row variant of the series tile, used when the feed view
/// toggle is set to `list`. Same data as `SeriesCard` but laid out for
/// scanning: small cover thumbnail on the left, title + metadata
/// stacked vertically, badges in a row. No synopsis yet — the list
/// endpoint does not return one; the detail page is where the full
/// description lives.
export function SeriesListRow({ series }: { series: SeriesListItem }) {
  return (
    <Link
      to="/series/$id"
      params={{ id: String(series.id) }}
      style={{ textDecoration: "none", color: "inherit" }}
      data-testid={`series-row-${series.id}`}
    >
      <Paper withBorder radius="md" p="sm">
        <Group gap="md" wrap="nowrap" align="flex-start">
          <Box w={72} style={{ flexShrink: 0 }}>
            <AspectRatio ratio={3 / 4}>
              <Image
                src={series.coverUrl ?? COVER_PLACEHOLDER}
                fallbackSrc={COVER_PLACEHOLDER}
                alt={series.canonicalTitle}
                loading="lazy"
                radius="sm"
              />
            </AspectRatio>
          </Box>
          <Stack gap={4} style={{ minWidth: 0, flex: 1 }}>
            <Title order={5} lineClamp={1} title={series.canonicalTitle}>
              {series.canonicalTitle}
            </Title>
            <Group gap={4} wrap="wrap">
              {series.kind && (
                <Badge size="xs" variant="light" color="blue">
                  {series.kind}
                </Badge>
              )}
              {series.status && (
                <Badge size="xs" variant="light" color="gray">
                  {series.status}
                </Badge>
              )}
              {typeof series.year === "number" && (
                <Badge size="xs" variant="default">
                  {series.year}
                </Badge>
              )}
              {series.owned && (
                <Badge size="xs" variant="filled" color="green">
                  owned
                </Badge>
              )}
            </Group>
            <Text size="xs" c="dimmed">
              last release {formatRelative(series.lastReleaseAt)} • first seen{" "}
              {formatRelative(series.firstSeenAt)}
            </Text>
          </Stack>
        </Group>
      </Paper>
    </Link>
  );
}
