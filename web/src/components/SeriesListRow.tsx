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

const MAX_GENRE_CHIPS = 4;
const MAX_TAG_CHIPS = 4;

/// Horizontal row variant of the series tile, used when the feed view
/// toggle is set to `list`. Trades grid density for scannability: a
/// larger cover on the left, title + badges on top, a clamped synopsis,
/// then a row of genre and tag chips. The list endpoint now returns the
/// description + the normalized genre/tag arrays so this stays a single
/// query.
export function SeriesListRow({ series }: { series: SeriesListItem }) {
  const genres = series.genres ?? [];
  const tags = series.tags ?? [];
  const genreOverflow = Math.max(0, genres.length - MAX_GENRE_CHIPS);
  const tagOverflow = Math.max(0, tags.length - MAX_TAG_CHIPS);
  return (
    <Link
      to="/series/$id"
      params={{ id: String(series.id) }}
      style={{ textDecoration: "none", color: "inherit" }}
      data-testid={`series-row-${series.id}`}
    >
      <Paper withBorder radius="md" p="md">
        <Group gap="md" wrap="nowrap" align="flex-start">
          <Box w={120} style={{ flexShrink: 0 }}>
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
          <Stack gap={6} style={{ minWidth: 0, flex: 1 }}>
            <Group gap="xs" align="baseline" wrap="wrap">
              <Title
                order={5}
                lineClamp={1}
                title={series.canonicalTitle}
                style={{ minWidth: 0 }}
              >
                {series.canonicalTitle}
              </Title>
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
            {series.description && (
              <Text size="sm" c="dimmed" lineClamp={3}>
                {series.description}
              </Text>
            )}
            {(genres.length > 0 || tags.length > 0) && (
              <Group gap={4} wrap="wrap">
                {genres.slice(0, MAX_GENRE_CHIPS).map((g) => (
                  <Badge key={`g-${g}`} size="xs" variant="light" color="grape">
                    {g}
                  </Badge>
                ))}
                {genreOverflow > 0 && (
                  <Badge size="xs" variant="light" color="grape">
                    +{genreOverflow}
                  </Badge>
                )}
                {tags.slice(0, MAX_TAG_CHIPS).map((t) => (
                  <Badge
                    key={`t-${t}`}
                    size="xs"
                    variant="outline"
                    color="gray"
                  >
                    {t}
                  </Badge>
                ))}
                {tagOverflow > 0 && (
                  <Badge size="xs" variant="outline" color="gray">
                    +{tagOverflow}
                  </Badge>
                )}
              </Group>
            )}
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
