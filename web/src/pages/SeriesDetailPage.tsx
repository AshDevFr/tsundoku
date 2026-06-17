import {
  ActionIcon,
  Alert,
  Anchor,
  AspectRatio,
  Badge,
  Box,
  Button,
  Card,
  Center,
  Checkbox,
  Container,
  CopyButton,
  Flex,
  Grid,
  Group,
  Image,
  Loader,
  Modal,
  SegmentedControl,
  Stack,
  Text,
  Title,
  Tooltip,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { Link, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import {
  useRefreshSeriesMetadata,
  useSendToClient,
  useSetIgnoreCompletion,
  useSetWishlisted,
} from "@/api/mutations";
import {
  type ReleaseDto,
  useDownloadStatus,
  useSeriesDetail,
  useSeriesReleases,
} from "@/api/queries";
import {
  coverProxyForSeries,
  formatAbsolute,
  formatRelative,
  nyaaSearchUrl,
  providerUrl,
} from "@/api/utils";
import { CodexBadge } from "@/components/CodexBadge";
import { EditSeriesModal } from "@/components/EditSeriesModal";
import {
  ExtractedLinks,
  InformationLink,
  ReleaseDescription,
  ReleaseFiles,
} from "@/components/ReleaseDetails";
import {
  LinkExistingPanel,
  ProviderSearchPanel,
} from "@/components/ReleaseLinking";
import { SendToClientButton, SentBadge } from "@/components/SendToClientButton";
import { useIsMobile } from "@/hooks/useIsMobile";
import { seriesDetailRoute } from "@/router";
import { useAdminAuth } from "@/stores/auth";
import type { FilterSearch } from "@/stores/filters";

const COVER_PLACEHOLDER =
  "data:image/svg+xml;utf8,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 3 4%22%3E%3Crect width=%223%22 height=%224%22 fill=%22%23ced4da%22/%3E%3C/svg%3E";

// Tags can number in the hundreds; collapse to a manageable count with a
// click-to-expand affordance so the detail page isn't dominated by the list.
const MAX_VISIBLE_TAGS = 30;

// `available/published` count label for a metric, e.g. `vol 111/114`. Falls
// back to available-only (`vol 111`) when the provider has no published total,
// or published-only (`vol 114`) when no release has been observed yet; null
// when neither count exists.
function spanCount(
  prefix: string,
  available: number | null | undefined,
  total: number | null | undefined,
): string | null {
  const a = typeof available === "number" ? available : null;
  const t = typeof total === "number" ? total : null;
  if (a !== null && t !== null) return `${prefix} ${a}/${t}`;
  if (a !== null) return `${prefix} ${a}`;
  if (t !== null) return `${prefix} ${t}`;
  return null;
}

// A release can be bulk-sent when it carries a magnet or `.torrent`.
// Already-sent releases stay selectable so the operator can deliberately
// re-send one (e.g. it was removed from the client); the "Sent" badge marks
// them, mirroring the per-release Send button which also allows re-sending.
function isSendable(r: ReleaseDto): boolean {
  return Boolean(r.magnet) || Boolean(r.torrentUrl);
}

// Group releases by source (kind/name), preserving first-seen order. Shared by
// the rendered list and the shift-select range so the two never drift.
function groupReleases(items: ReleaseDto[]): Map<string, ReleaseDto[]> {
  const groups = new Map<string, ReleaseDto[]>();
  for (const r of items) {
    const key = `${r.sourceKind}:${r.sourceName}`;
    const arr = groups.get(key);
    if (arr) arr.push(r);
    else groups.set(key, [r]);
  }
  return groups;
}

// The sendable release ids in the exact order they render (group by group),
// so a shift-click can select the contiguous visible range between two rows.
function orderedSendableIds(items: ReleaseDto[]): string[] {
  const out: string[] = [];
  for (const rs of groupReleases(items).values()) {
    for (const r of rs) if (isSendable(r)) out.push(r.id);
  }
  return out;
}

// Threaded into the release list when the bulk-send affordance is active. Each
// row consults it to render (or skip) its selection checkbox. `range` is true
// when the click was shift-held (select the span since the last click).
type BulkSelect = {
  selected: Set<string>;
  onToggle: (id: string, range: boolean) => void;
};

export function SeriesDetailPage() {
  const { id: idStr } = seriesDetailRoute.useParams();
  const id = Number(idStr);
  const detail = useSeriesDetail(Number.isFinite(id) ? id : undefined);
  const releases = useSeriesReleases(Number.isFinite(id) ? id : undefined);
  const isAdmin = useAdminAuth((s) => Boolean(s.token));
  const refresh = useRefreshSeriesMetadata();
  const ignoreToggle = useSetIgnoreCompletion();
  const wishlistToggle = useSetWishlisted();
  const [tagsExpanded, setTagsExpanded] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const navigate = useNavigate();
  const downloadStatus = useDownloadStatus();
  const send = useSendToClient();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  // The last row toggled, anchoring a shift-click range select.
  const [anchorId, setAnchorId] = useState<string | null>(null);
  const [bulkSending, setBulkSending] = useState(false);

  // Jump to the feed pre-filtered by a clicked genre/tag badge. "any" mode and
  // page 1 match how the filter panel seeds a fresh single-value selection.
  const filterFeedBy = (next: FilterSearch) =>
    navigate({ to: "/", search: () => ({ ...next, page: 1 }) });

  // The bulk "send to client" affordance shares the SendToClientButton's
  // gating: admin + integration enabled. A release is selectable only when it
  // has something to send and hasn't already been sent.
  const bulkEnabled = isAdmin && Boolean(downloadStatus.data?.enabled);

  // Toggle one release, or — on a shift-click with a prior anchor — set the
  // whole visible span between the anchor and this row to the clicked row's new
  // state (standard range-select). Either way, this row becomes the new anchor.
  const toggleSelected = (id: string, range: boolean) => {
    setSelected((prev) => {
      const next = new Set(prev);
      const willSelect = !prev.has(id);
      const order = orderedSendableIds(releases.data?.items ?? []);
      const a = anchorId ? order.indexOf(anchorId) : -1;
      const b = order.indexOf(id);
      if (range && a !== -1 && b !== -1) {
        const [lo, hi] = a < b ? [a, b] : [b, a];
        for (let i = lo; i <= hi; i += 1) {
          if (willSelect) next.add(order[i]);
          else next.delete(order[i]);
        }
      } else if (willSelect) {
        next.add(id);
      } else {
        next.delete(id);
      }
      return next;
    });
    setAnchorId(id);
  };

  const clearSelection = () => {
    setSelected(new Set());
    setAnchorId(null);
  };

  // Send each selected release through the existing per-release endpoint, one at
  // a time (gentle on the seedbox XML-RPC), and report a single aggregated
  // result. The loop never throws: a failed send is tallied, not fatal.
  const handleBulkSend = async () => {
    const ids = [...selected];
    if (ids.length === 0) return;
    setBulkSending(true);
    let ok = 0;
    let failed = 0;
    for (const releaseId of ids) {
      try {
        await send.mutateAsync({ releaseId, body: {} });
        ok += 1;
      } catch {
        failed += 1;
      }
    }
    setBulkSending(false);
    clearSelection();
    notifications.show({
      color: failed === 0 ? "blue" : ok === 0 ? "red" : "yellow",
      message:
        failed === 0 ? `${ok} sent to client` : `${ok} sent, ${failed} failed`,
    });
  };

  const handleRefresh = () => {
    if (!Number.isFinite(id)) return;
    refresh.mutate(id, {
      onSuccess: () =>
        notifications.show({
          color: "blue",
          message: "Series metadata refreshed",
        }),
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Refresh failed",
          message: (e as Error).message,
        }),
    });
  };

  const handleToggleWishlist = (wishlisted: boolean) => {
    if (!Number.isFinite(id)) return;
    wishlistToggle.mutate(
      { id, wishlisted },
      {
        onSuccess: () =>
          notifications.show({
            color: "blue",
            message: wishlisted ? "Added to wishlist" : "Removed from wishlist",
          }),
        onError: (e) =>
          notifications.show({
            color: "red",
            title: "Update failed",
            message: (e as Error).message,
          }),
      },
    );
  };

  const handleToggleIgnore = (ignore: boolean) => {
    if (!Number.isFinite(id)) return;
    ignoreToggle.mutate(
      { id, ignore },
      {
        onSuccess: () =>
          notifications.show({
            color: "blue",
            message: ignore
              ? "Completion tracking muted for this series"
              : "Completion tracking resumed",
          }),
        onError: (e) =>
          notifications.show({
            color: "red",
            title: "Update failed",
            message: (e as Error).message,
          }),
      },
    );
  };

  if (detail.isLoading) {
    return (
      <Center py="xl">
        <Loader />
      </Center>
    );
  }

  if (detail.isError) {
    return (
      <Container py="lg">
        <Alert color="red" title="Failed to load series">
          {(detail.error as Error)?.message ?? "Unknown error"}
        </Alert>
        <Button
          renderRoot={(props) => (
            <Link to="/" search={(prev) => prev} {...props} />
          )}
          mt="md"
          variant="subtle"
        >
          ← Back to feed
        </Button>
      </Container>
    );
  }

  if (!detail.data) return null;
  const s = detail.data;
  const codexIgnored = s.codex?.status === "ignored";

  const spanParts = [
    spanCount("vol", s.highestVolume, s.totalVolumes),
    spanCount("ch", s.highestChapter, s.totalChapters),
  ].filter(Boolean);
  const hasAvailable =
    typeof s.highestVolume === "number" || typeof s.highestChapter === "number";

  return (
    // Tighter horizontal gutter on mobile so the release cards use more of the
    // screen; restores the standard `md` gutter from `sm` up.
    <Container size="xl" py="lg" px={{ base: "xs", sm: "md" }}>
      <Button
        renderRoot={(props) => (
          <Link to="/" search={(prev) => prev} {...props} />
        )}
        mb="md"
        variant="subtle"
        size="xs"
      >
        ← Back to feed
      </Button>

      <Grid gap="xl">
        <Grid.Col span={{ base: 12, sm: 4, md: 3 }}>
          <AspectRatio ratio={3 / 4}>
            <Image
              src={s.coverUrl ? coverProxyForSeries(s.id) : COVER_PLACEHOLDER}
              fallbackSrc={COVER_PLACEHOLDER}
              alt={s.canonicalTitle}
              radius="md"
            />
          </AspectRatio>
        </Grid.Col>

        <Grid.Col span={{ base: 12, sm: 8, md: 9 }}>
          <Stack gap="sm">
            <Title order={2}>{s.canonicalTitle}</Title>
            {s.alternateTitles.length > 0 && (
              <Group gap={6}>
                {s.alternateTitles.map((t) => (
                  <Badge
                    key={t}
                    size="sm"
                    radius="sm"
                    variant="default"
                    tt="none"
                    fw={400}
                    title={t}
                  >
                    {t}
                  </Badge>
                ))}
              </Group>
            )}
            <Group gap="xs">
              {s.kind && <Badge variant="light">{s.kind}</Badge>}
              {s.status && (
                <Badge variant="light" color="gray">
                  {s.status}
                </Badge>
              )}
              {typeof s.year === "number" && (
                <Badge variant="default">{s.year}</Badge>
              )}
              {typeof s.rating === "number" && (
                <Tooltip label="Provider rating, normalized to 0-10">
                  <Badge variant="light" color="yellow">
                    ★ {s.rating.toFixed(1)}
                  </Badge>
                </Tooltip>
              )}
              {isAdmin && s.codex && <CodexBadge codex={s.codex} asLink />}
              {s.metadataSource === "manual" && (
                <Badge variant="light" color="grape">
                  manual
                </Badge>
              )}
            </Group>

            {spanParts.length > 0 && (
              <Text size="sm">
                {spanParts.join(" · ")}{" "}
                <Text component="span" c="dimmed" size="xs">
                  {hasAvailable ? "available / published" : "published"}
                </Text>
              </Text>
            )}

            {s.genres.length > 0 && (
              <Group gap={4}>
                {s.genres.map((g) => (
                  <Badge
                    key={g}
                    size="sm"
                    variant="outline"
                    color="grape"
                    style={{ cursor: "pointer" }}
                    onClick={() =>
                      filterFeedBy({ genres: [g], genresMode: "any" })
                    }
                    data-testid={`genre-badge-${g}`}
                  >
                    {g}
                  </Badge>
                ))}
              </Group>
            )}

            {s.tags.length > 0 && (
              <Group gap={4}>
                {(tagsExpanded
                  ? s.tags
                  : s.tags.slice(0, MAX_VISIBLE_TAGS)
                ).map((t) => (
                  <Badge
                    key={t}
                    size="sm"
                    variant="light"
                    color="blue"
                    style={{ cursor: "pointer" }}
                    onClick={() => filterFeedBy({ tags: [t], tagsMode: "any" })}
                    data-testid={`tag-badge-${t}`}
                  >
                    {t}
                  </Badge>
                ))}
                {s.tags.length > MAX_VISIBLE_TAGS && (
                  <Badge
                    size="sm"
                    variant="light"
                    color="gray"
                    style={{ cursor: "pointer" }}
                    onClick={() => setTagsExpanded((v) => !v)}
                  >
                    {tagsExpanded
                      ? "show less"
                      : `+${s.tags.length - MAX_VISIBLE_TAGS} more`}
                  </Badge>
                )}
              </Group>
            )}

            {s.description && (
              <Text size="sm" style={{ whiteSpace: "pre-line" }}>
                {s.description}
              </Text>
            )}

            <Box>
              <Group gap="xs" wrap="wrap" align="center">
                <Text size="sm" c="dimmed">
                  First seen {formatRelative(s.firstSeenAt)} · last release{" "}
                  {formatRelative(s.lastReleaseAt)} · metadata{" "}
                  {s.metadataSource} ({formatRelative(s.metadataFetchedAt)})
                </Text>
                <Tooltip label="Search Nyaa (English-translated manga) for this title.">
                  <Button
                    size="compact-xs"
                    variant="subtle"
                    color="gray"
                    component="a"
                    href={nyaaSearchUrl(s.canonicalTitle)}
                    target="_blank"
                    rel="noreferrer noopener"
                    data-testid="search-nyaa"
                  >
                    ⌕ Search on Nyaa
                  </Button>
                </Tooltip>
                {isAdmin && (
                  <Tooltip label="Clip this series to your wishlist (a curated 'download later' list). Independent of Codex ownership; remove it the same way.">
                    <Button
                      size="compact-xs"
                      variant={s.wishlisted ? "light" : "subtle"}
                      color={s.wishlisted ? "yellow" : "gray"}
                      onClick={() => handleToggleWishlist(!s.wishlisted)}
                      loading={wishlistToggle.isPending}
                      data-testid="toggle-wishlist"
                    >
                      {s.wishlisted ? "★ On wishlist" : "☆ Add to wishlist"}
                    </Button>
                  </Tooltip>
                )}
                {isAdmin && s.metadataSource === "manual" && (
                  <Tooltip label="Edit this manual series' title and metadata.">
                    <Button
                      size="compact-xs"
                      variant="subtle"
                      color="gray"
                      onClick={() => setEditOpen(true)}
                      data-testid="edit-series"
                    >
                      ✎ Edit
                    </Button>
                  </Tooltip>
                )}
                {isAdmin && (
                  <Tooltip label="Re-fetch metadata from the active provider and overwrite this series.">
                    <Button
                      size="compact-xs"
                      variant="subtle"
                      color="gray"
                      onClick={handleRefresh}
                      loading={refresh.isPending}
                      data-testid="refresh-series-metadata"
                    >
                      ↻ Refresh
                    </Button>
                  </Tooltip>
                )}
                {isAdmin && s.codex && (
                  <Tooltip label="Mute the 'behind' signal for this series (e.g. read in omnibus). It stays owned; its completion just isn't tracked.">
                    <Button
                      size="compact-xs"
                      variant="subtle"
                      color="gray"
                      onClick={() => handleToggleIgnore(!codexIgnored)}
                      loading={ignoreToggle.isPending}
                      data-testid="toggle-ignore-completion"
                    >
                      {codexIgnored
                        ? "◎ Resume tracking"
                        : "⊘ Ignore completion"}
                    </Button>
                  </Tooltip>
                )}
              </Group>
            </Box>

            {s.externalIds.length > 0 && (
              <Group gap="xs" mt="xs">
                {s.externalIds.map((x) => {
                  const href = providerUrl(x.provider, x.externalId);
                  return (
                    <Badge
                      key={`${x.provider}-${x.externalId}`}
                      variant="dot"
                      color="blue"
                      component={href ? "a" : undefined}
                      {...(href
                        ? {
                            href,
                            target: "_blank",
                            rel: "noreferrer noopener",
                            style: {
                              cursor: "pointer",
                              textDecoration: "none",
                            },
                          }
                        : {})}
                    >
                      {x.provider}: {x.externalId}
                    </Badge>
                  );
                })}
              </Group>
            )}
          </Stack>
        </Grid.Col>
      </Grid>

      <Box mt="xl">
        <Group justify="space-between" align="center" mb="sm">
          <Title order={3}>Releases</Title>
          {bulkEnabled && selected.size > 0 && (
            <Group gap="xs">
              <Text size="sm" c="dimmed">
                {selected.size} selected
              </Text>
              <Button
                size="compact-sm"
                color="blue"
                onClick={handleBulkSend}
                loading={bulkSending}
                data-testid="bulk-send"
              >
                Send {selected.size} to client
              </Button>
              <Button
                size="compact-sm"
                variant="subtle"
                color="gray"
                onClick={clearSelection}
                data-testid="bulk-clear"
              >
                Clear
              </Button>
            </Group>
          )}
        </Group>
        {releases.isLoading && (
          <Center py="md">
            <Loader size="sm" />
          </Center>
        )}
        {releases.data && (
          <ReleaseList
            items={releases.data.items}
            bulk={
              bulkEnabled ? { selected, onToggle: toggleSelected } : undefined
            }
          />
        )}
      </Box>

      {editOpen && (
        <EditSeriesModal series={s} onClose={() => setEditOpen(false)} />
      )}
    </Container>
  );
}

function ReleaseList({
  items,
  bulk,
}: {
  items: ReleaseDto[];
  bulk?: BulkSelect;
}) {
  if (items.length === 0) {
    return (
      <Text c="dimmed" size="sm">
        No releases recorded for this series yet.
      </Text>
    );
  }

  // Group by source (kind/name) so the user can scan per-uploader streams.
  const groups = new Map<string, ReleaseDto[]>();
  for (const r of items) {
    const key = `${r.sourceKind}:${r.sourceName}`;
    const arr = groups.get(key);
    if (arr) arr.push(r);
    else groups.set(key, [r]);
  }

  return (
    <Stack gap="md">
      {Array.from(groups.entries()).map(([key, rs]) => (
        <Box key={key}>
          <Group gap="xs" mb={6}>
            <Badge color="indigo" variant="light">
              {key}
            </Badge>
            <Text size="xs" c="dimmed">
              {rs.length} release{rs.length === 1 ? "" : "s"}
            </Text>
          </Group>
          <Stack gap={6}>
            {rs.map((r) => (
              <ReleaseRow
                key={r.id}
                release={r}
                bulkActive={Boolean(bulk)}
                select={
                  bulk && isSendable(r)
                    ? {
                        checked: bulk.selected.has(r.id),
                        onToggle: (range: boolean) =>
                          bulk.onToggle(r.id, range),
                      }
                    : undefined
                }
              />
            ))}
          </Stack>
        </Box>
      ))}
    </Stack>
  );
}

function CopyLinkButton({ value, label }: { value: string; label: string }) {
  return (
    <CopyButton value={value} timeout={1500}>
      {({ copied, copy }) => (
        <Tooltip label={copied ? "Copied!" : `Copy ${label}`} withArrow>
          <ActionIcon
            size="xs"
            variant="subtle"
            color={copied ? "teal" : "gray"}
            onClick={copy}
            aria-label={`Copy ${label}`}
          >
            {copied ? (
              <svg
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                width="14"
                height="14"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.4"
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true"
                role="presentation"
              >
                <title>Copied</title>
                <path d="M20 6L9 17l-5-5" />
              </svg>
            ) : (
              <svg
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                width="14"
                height="14"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true"
                role="presentation"
              >
                <title>Copy</title>
                <rect x="9" y="9" width="11" height="11" rx="2" />
                <path d="M5 15V5a2 2 0 0 1 2-2h10" />
              </svg>
            )}
          </ActionIcon>
        </Tooltip>
      )}
    </CopyButton>
  );
}

function ReleaseRow({
  release,
  select,
  bulkActive,
}: {
  release: ReleaseDto;
  select?: { checked: boolean; onToggle: (range: boolean) => void };
  /// True when the bulk-send affordance is on for the list. Rows that aren't
  /// selectable (already sent, or nothing to send) still reserve the checkbox
  /// slot with a disabled box so every row stays aligned.
  bulkActive?: boolean;
}) {
  // The relink ("Move") action calls a write endpoint, so only offer it when
  // an admin token is present — the series detail page is otherwise a public
  // browse view.
  const isAdmin = useAdminAuth((s) => Boolean(s.token));
  const [moveOpen, { open: openMove, close: closeMove }] = useDisclosure(false);

  return (
    <Card withBorder padding="xs" radius="sm">
      {/* Side-by-side on desktop; stacks on mobile so the long release title
          gets the full width instead of being crushed by the action buttons. */}
      <Flex
        direction={{ base: "column", sm: "row" }}
        justify="space-between"
        align={{ base: "stretch", sm: "flex-start" }}
        gap="xs"
      >
        <Group
          wrap="nowrap"
          align="flex-start"
          gap="xs"
          style={{ minWidth: 0, flex: 1 }}
        >
          {select ? (
            <Checkbox
              checked={select.checked}
              // Toggling is driven from onClick so we can read the shift key for
              // range select (the change event's nativeEvent doesn't carry it).
              // The controlled `checked` resyncs the box after the click.
              onChange={() => {}}
              onClick={(e) => select.onToggle(e.shiftKey)}
              aria-label={`Select release ${release.title}`}
              data-testid={`select-release-${release.id}`}
              mt={2}
            />
          ) : bulkActive ? (
            // Already sent / nothing to send: keep the slot so rows align.
            <Checkbox disabled aria-label="Release not selectable" mt={2} />
          ) : null}
          <Stack gap={2} style={{ minWidth: 0, flex: 1 }}>
            <Anchor
              href={release.link}
              target="_blank"
              rel="noreferrer noopener"
              size="sm"
              lineClamp={1}
              title={release.title}
            >
              {release.title}
            </Anchor>
            <Group gap={4} wrap="wrap">
              {release.formats.map((f) => (
                <Badge key={f} size="xs" variant="outline">
                  {f}
                </Badge>
              ))}
              <Text
                size="xs"
                c="dimmed"
                title={formatAbsolute(release.postedAt)}
              >
                posted {formatRelative(release.postedAt)}
              </Text>
              {release.resolutionPath && (
                <Badge size="xs" variant="dot" color="teal">
                  {release.resolutionPath}
                </Badge>
              )}
              <SentBadge release={release} />
            </Group>
          </Stack>
        </Group>
        <Group gap={8} wrap="wrap" align="center">
          {release.magnet && (
            <Group gap={2} wrap="nowrap" align="center">
              <Anchor href={release.magnet} size="xs" rel="noreferrer">
                magnet
              </Anchor>
              <CopyLinkButton value={release.magnet} label="magnet link" />
            </Group>
          )}
          {release.torrentUrl && (
            <Group gap={2} wrap="nowrap" align="center">
              <Anchor
                href={release.torrentUrl}
                size="xs"
                target="_blank"
                rel="noreferrer noopener"
              >
                .torrent
              </Anchor>
              <CopyLinkButton
                value={release.torrentUrl}
                label=".torrent link"
              />
            </Group>
          )}
          {release.ddlUrl && (
            <Group gap={2} wrap="nowrap" align="center">
              <Anchor
                href={release.ddlUrl}
                size="xs"
                target="_blank"
                rel="noreferrer noopener"
              >
                DDL
              </Anchor>
              <CopyLinkButton value={release.ddlUrl} label="DDL link" />
            </Group>
          )}
          {isAdmin && (
            <Tooltip label="Wrong series? Move this release to the correct one.">
              <Button
                size="compact-xs"
                variant="light"
                color="orange"
                onClick={openMove}
                data-testid={`move-release-${release.id}`}
              >
                Move
              </Button>
            </Tooltip>
          )}
          <SendToClientButton release={release} />
        </Group>
      </Flex>

      <Stack gap={6} mt={6}>
        <ExtractedLinks links={release.extractedLinks} />
        <InformationLink url={release.informationUrl} />
        <ReleaseDescription body={release.descriptionHtml} />
        <ReleaseFiles files={release.files} />
      </Stack>

      {moveOpen && <MoveReleaseModal release={release} onClose={closeMove} />}
    </Card>
  );
}

/// Relink a release that landed on the wrong series. Hosts the same catalog
/// and provider search the review queue uses, behind a tab toggle. The link
/// endpoint re-points the release regardless of its current status, so a
/// successful pick moves it off this series; query invalidation then drops it
/// from the list.
function MoveReleaseModal({
  release,
  onClose,
}: {
  release: ReleaseDto;
  onClose: () => void;
}) {
  const [tab, setTab] = useState("catalog");
  const isMobile = useIsMobile();

  return (
    <Modal
      opened
      onClose={onClose}
      title="Move release to another series"
      size="lg"
      centered
      fullScreen={isMobile}
    >
      <Stack gap="md">
        <Text size="sm" c="dimmed" lineClamp={2} title={release.title}>
          {release.title}
        </Text>
        <SegmentedControl
          value={tab}
          onChange={setTab}
          fullWidth
          data={[
            { label: "Catalog", value: "catalog" },
            { label: "Provider search", value: "provider" },
          ]}
        />
        {tab === "catalog" ? (
          <LinkExistingPanel
            releaseId={release.id}
            seedQuery={release.title}
            onLinked={onClose}
          />
        ) : (
          <ProviderSearchPanel
            releaseId={release.id}
            seedQuery={release.title}
            onLinked={onClose}
          />
        )}
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Close
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}
