import {
  ActionIcon,
  AspectRatio,
  Badge,
  Card,
  Divider,
  Group,
  Image,
  Stack,
  Text,
  Title,
  Tooltip,
} from "@mantine/core";
import { Link } from "@tanstack/react-router";
import type { MouseEvent } from "react";
import { useSetWishlisted } from "@/api/mutations";
import type { SeriesListItem } from "@/api/queries";
import { coverProxyForSeries, formatRelative } from "@/api/utils";
import { CodexBadge, codexBorderColor } from "@/components/CodexBadge";
import { useAdminAuth } from "@/stores/auth";

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

export function SeriesCard({
  series,
  codexSynced = false,
}: {
  series: SeriesListItem;
  /// Whether at least one Codex sweep has succeeded (from the list page's
  /// `codexSyncedAt`). Gates the Codex badge so a pre-first-sync admin sees
  /// no stale/empty state.
  codexSynced?: boolean;
}) {
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
  // Accent the whole tile only for the actionable Codex state (`behind`), so
  // series with new content to grab pop beyond the small badge. Already-handled
  // (`complete`/`present`) series return no color and stay quiet.
  const codexBorder =
    codexSynced && series.codex
      ? codexBorderColor(series.codex.status)
      : undefined;

  // Wishlist clip is admin-only — the `wishlisted` flag is blanked for
  // non-admins server-side, so the control only renders with a token.
  const isAdmin = useAdminAuth((s) => Boolean(s.token));
  const toggleWishlist = useSetWishlisted();
  const clip = (e: MouseEvent) => {
    // The whole card is a <Link>; keep the clip from navigating to detail.
    e.preventDefault();
    e.stopPropagation();
    toggleWishlist.mutate({ id: series.id, wishlisted: !series.wishlisted });
  };

  return (
    <Link
      to="/series/$id"
      params={{ id: String(series.id) }}
      // Carry the current feed filters into the detail route so its
      // "Back to feed" link can restore them.
      search={(prev) => prev}
      style={{ textDecoration: "none", color: "inherit", height: "100%" }}
      data-testid={`series-card-${series.id}`}
    >
      <Card
        shadow="sm"
        padding="sm"
        radius="md"
        withBorder
        h="100%"
        style={
          codexBorder ? { borderColor: codexBorder, borderWidth: 2 } : undefined
        }
      >
        <Card.Section pos="relative">
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
          {isAdmin && (
            <Tooltip
              label={
                series.wishlisted ? "Remove from wishlist" : "Add to wishlist"
              }
              withinPortal
            >
              <ActionIcon
                variant={series.wishlisted ? "filled" : "default"}
                color="yellow"
                radius="xl"
                size="md"
                pos="absolute"
                top={6}
                right={6}
                onClick={clip}
                loading={toggleWishlist.isPending}
                aria-label={
                  series.wishlisted ? "Remove from wishlist" : "Add to wishlist"
                }
                data-testid={`wishlist-toggle-${series.id}`}
              >
                {series.wishlisted ? "★" : "☆"}
              </ActionIcon>
            </Tooltip>
          )}
        </Card.Section>
        <Stack gap={4} mt="xs">
          {/* Reserve a constant two-line height so single-line titles don't
              pull the date / divider / badge rows up: keeps those rows aligned
              across every card in the grid. `lh` * 2 == `minHeight`. The title
              clamps to two lines, so a hover tooltip surfaces the full title
              plus rating and a synopsis excerpt the card has no room for. */}
          <Tooltip
            multiline
            w={320}
            withinPortal
            openDelay={250}
            label={
              <Stack gap={2}>
                <Text fw={600} size="sm">
                  {series.canonicalTitle}
                </Text>
                {typeof series.rating === "number" && (
                  <Text size="xs">★ {series.rating.toFixed(1)} / 10</Text>
                )}
                {series.description && (
                  <Text size="xs" lineClamp={8}>
                    {series.description}
                  </Text>
                )}
              </Stack>
            }
          >
            <Title
              order={5}
              lineClamp={2}
              lh={1.25}
              style={{ minHeight: "2.5em" }}
            >
              {series.canonicalTitle}
            </Title>
          </Tooltip>
          {/* Ownership rides the (otherwise near-empty) timestamp line,
              right-aligned, so it reads as a distinct signal rather than
              getting lost among the metadata badges below. */}
          <Group gap={4} justify="space-between" wrap="nowrap">
            <Text size="xs" c="dimmed">
              {formatRelative(series.lastReleaseAt)}
            </Text>
            {codexSynced && series.codex && <CodexBadge codex={series.codex} />}
          </Group>
          {/* Separate the date/ownership lane from the metadata badge block. */}
          <Divider my={6} />
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
