import {
  AspectRatio,
  Badge,
  Box,
  Checkbox,
  Group,
  Image,
  Paper,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import type { SeriesListItem } from "@/api/queries";
import { coverProxyForSeries, formatRelative } from "@/api/utils";
import { CodexBadge, codexBorderColor } from "@/components/CodexBadge";
import {
  type SeriesSelectionProps,
  spanBadgeLabel,
} from "@/components/SeriesCard";

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
export function SeriesListRow({
  series,
  codexSynced = false,
  selection,
}: {
  series: SeriesListItem;
  /// See `SeriesCard` — gates the Codex badge on a successful first sweep.
  codexSynced?: boolean;
  /// See `SeriesCard` — bulk-selection wiring, admin pages only.
  selection?: SeriesSelectionProps;
}) {
  const genres = series.genres ?? [];
  const tags = series.tags ?? [];
  const genreOverflow = Math.max(0, genres.length - MAX_GENRE_CHIPS);
  const tagOverflow = Math.max(0, tags.length - MAX_TAG_CHIPS);
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
  const codexBorder =
    codexSynced && series.codex
      ? codexBorderColor(series.codex.status)
      : undefined;
  // See `SeriesCard` — invisible until hover, forced visible while a
  // selection is active on the page (or this row is selected).
  const [hovered, setHovered] = useState(false);
  const selectionVisible =
    Boolean(selection) &&
    (hovered || Boolean(selection?.active) || Boolean(selection?.selected));
  return (
    <Link
      to="/series/$id"
      params={{ id: String(series.id) }}
      // Carry the current feed filters into the detail route so its
      // "Back to feed" link can restore them.
      search={(prev) => prev}
      style={{ textDecoration: "none", color: "inherit" }}
      data-testid={`series-row-${series.id}`}
      onMouseEnter={selection ? () => setHovered(true) : undefined}
      onMouseLeave={selection ? () => setHovered(false) : undefined}
    >
      <Paper
        withBorder
        radius="md"
        p="md"
        style={
          codexBorder ? { borderColor: codexBorder, borderWidth: 2 } : undefined
        }
      >
        <Group gap="md" wrap="nowrap" align="flex-start">
          {selection && (
            <Box
              data-selection-overlay
              style={{
                alignSelf: "center",
                opacity: selectionVisible ? 1 : 0,
                transition: "opacity 120ms ease",
              }}
              onClick={(e) => {
                // The whole row is a <Link>; selecting must not navigate.
                e.preventDefault();
                e.stopPropagation();
                selection.onToggle(e);
              }}
            >
              <Checkbox
                checked={selection.selected}
                onChange={() => {
                  // Toggled by the wrapping Box's onClick (which sees
                  // shiftKey); selection state lives in the page.
                }}
                // See SeriesCard: pointer-transparent so the preventDefault-ed
                // native checkbox toggle can't desync the visual state.
                style={{ pointerEvents: "none" }}
                size="md"
                aria-label={
                  selection.selected
                    ? `Deselect ${series.canonicalTitle}`
                    : `Select ${series.canonicalTitle}`
                }
                data-testid={`series-select-${series.id}`}
              />
            </Box>
          )}
          <Box w={120} style={{ flexShrink: 0 }}>
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
              {codexSynced && series.codex && (
                <CodexBadge codex={series.codex} />
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
