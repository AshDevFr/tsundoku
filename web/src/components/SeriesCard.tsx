import {
  AspectRatio,
  Badge,
  Card,
  Group,
  Image,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { Link } from "@tanstack/react-router";
import type { SeriesListItem } from "@/api/queries";
import { coverProxyForSeries, formatRelative } from "@/api/utils";

const COVER_PLACEHOLDER =
  "data:image/svg+xml;utf8,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 3 4%22%3E%3Crect width=%223%22 height=%224%22 fill=%22%23ced4da%22/%3E%3C/svg%3E";

/// `available/total` count badge text, e.g. `vol 5/11`. Renders only when an
/// observed (available) count exists; the published total is appended only
/// when the provider knows it, so a series with releases but no metadata
/// total still shows `vol 5`. Returns `null` when nothing is available yet.
export function spanBadgeLabel(
  prefix: string,
  available: number | null | undefined,
  total: number | null | undefined,
): string | null {
  if (typeof available !== "number") return null;
  return typeof total === "number"
    ? `${prefix} ${available}/${total}`
    : `${prefix} ${available}`;
}

export function SeriesCard({ series }: { series: SeriesListItem }) {
  const volLabel = spanBadgeLabel(
    "vol",
    series.highestVolume,
    series.totalVolumes,
  );
  const chLabel = spanBadgeLabel(
    "ch",
    series.highestChapter,
    series.totalChapters,
  );
  return (
    <Link
      to="/series/$id"
      params={{ id: String(series.id) }}
      style={{ textDecoration: "none", color: "inherit", height: "100%" }}
      data-testid={`series-card-${series.id}`}
    >
      <Card shadow="sm" padding="sm" radius="md" withBorder h="100%">
        <Card.Section>
          <AspectRatio ratio={3 / 4}>
            <Image
              src={
                series.coverUrl
                  ? coverProxyForSeries(series.id)
                  : COVER_PLACEHOLDER
              }
              fallbackSrc={COVER_PLACEHOLDER}
              alt={series.canonicalTitle}
              loading="lazy"
            />
          </AspectRatio>
        </Card.Section>
        <Stack gap={4} mt="xs">
          <Title order={5} lineClamp={2} title={series.canonicalTitle}>
            {series.canonicalTitle}
          </Title>
          <Text size="xs" c="dimmed">
            {formatRelative(series.lastReleaseAt)}
          </Text>
          <Group gap={4} mt={4} wrap="wrap">
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
            {series.metadataSource === "manual" && (
              <Badge size="xs" variant="light" color="grape">
                manual
              </Badge>
            )}
            {volLabel && (
              <Badge
                size="xs"
                variant="light"
                color="indigo"
                title="Highest volume available across linked releases / published total"
              >
                {volLabel}
              </Badge>
            )}
            {chLabel && (
              <Badge
                size="xs"
                variant="light"
                color="cyan"
                title="Highest chapter available across linked releases / published total"
              >
                {chLabel}
              </Badge>
            )}
            <Badge
              size="xs"
              variant="light"
              color={series.releaseCount === 0 ? "red" : "teal"}
              title={
                series.releaseCount === 0
                  ? "No releases linked to this series"
                  : `${series.releaseCount} release${series.releaseCount === 1 ? "" : "s"}`
              }
            >
              {series.releaseCount} rel
            </Badge>
          </Group>
        </Stack>
      </Card>
    </Link>
  );
}
