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
  Container,
  CopyButton,
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
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { useRefreshSeriesMetadata } from "@/api/mutations";
import {
  type ReleaseDto,
  useSeriesDetail,
  useSeriesReleases,
} from "@/api/queries";
import {
  coverProxyForSeries,
  formatAbsolute,
  formatRelative,
  providerUrl,
} from "@/api/utils";
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
import { spanBadgeLabel } from "@/components/SeriesCard";
import { seriesDetailRoute } from "@/router";
import { useAdminAuth } from "@/stores/auth";

const COVER_PLACEHOLDER =
  "data:image/svg+xml;utf8,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 3 4%22%3E%3Crect width=%223%22 height=%224%22 fill=%22%23ced4da%22/%3E%3C/svg%3E";

// Tags can number in the hundreds; collapse to a manageable count with a
// click-to-expand affordance so the detail page isn't dominated by the list.
const MAX_VISIBLE_TAGS = 30;

export function SeriesDetailPage() {
  const { id: idStr } = seriesDetailRoute.useParams();
  const id = Number(idStr);
  const detail = useSeriesDetail(Number.isFinite(id) ? id : undefined);
  const releases = useSeriesReleases(Number.isFinite(id) ? id : undefined);
  const isAdmin = useAdminAuth((s) => Boolean(s.token));
  const refresh = useRefreshSeriesMetadata();
  const [tagsExpanded, setTagsExpanded] = useState(false);

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
        <Button component={Link} to="/" mt="md" variant="subtle">
          ← Back to feed
        </Button>
      </Container>
    );
  }

  if (!detail.data) return null;
  const s = detail.data;

  return (
    <Container size="xl" py="lg">
      <Button component={Link} to="/" mb="md" variant="subtle" size="xs">
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
              {s.owned && (
                <Badge variant="filled" color="green">
                  owned
                </Badge>
              )}
              {s.metadataSource === "manual" && (
                <Badge variant="light" color="grape">
                  manual
                </Badge>
              )}
            </Group>

            {(typeof s.totalVolumes === "number" ||
              typeof s.totalChapters === "number") && (
              <Text size="sm">
                {typeof s.totalVolumes === "number" && `${s.totalVolumes} vol`}
                {typeof s.totalVolumes === "number" &&
                  typeof s.totalChapters === "number" &&
                  " · "}
                {typeof s.totalChapters === "number" && `${s.totalChapters} ch`}{" "}
                <Text component="span" c="dimmed" size="xs">
                  published
                </Text>
              </Text>
            )}

            {s.genres.length > 0 && (
              <Group gap={4}>
                {s.genres.map((g) => (
                  <Badge key={g} size="sm" variant="outline" color="grape">
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
                  <Badge key={t} size="sm" variant="light" color="blue">
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
              </Group>
              {(typeof s.highestVolume === "number" ||
                typeof s.highestChapter === "number") && (
                <Text size="sm" c="dimmed">
                  Available across releases:{" "}
                  {[
                    spanBadgeLabel("vol", s.highestVolume, s.totalVolumes),
                    spanBadgeLabel("ch", s.highestChapter, s.totalChapters),
                  ]
                    .filter(Boolean)
                    .join(", ")}{" "}
                  <Text component="span" size="xs">
                    (available/published total)
                  </Text>
                </Text>
              )}
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
        <Title order={3} mb="sm">
          Releases
        </Title>
        {releases.isLoading && (
          <Center py="md">
            <Loader size="sm" />
          </Center>
        )}
        {releases.data && <ReleaseList items={releases.data.items} />}
      </Box>
    </Container>
  );
}

function ReleaseList({ items }: { items: ReleaseDto[] }) {
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
              <ReleaseRow key={r.id} release={r} />
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

function ReleaseRow({ release }: { release: ReleaseDto }) {
  // The relink ("Move") action calls a write endpoint, so only offer it when
  // an admin token is present — the series detail page is otherwise a public
  // browse view.
  const isAdmin = useAdminAuth((s) => Boolean(s.token));
  const [moveOpen, { open: openMove, close: closeMove }] = useDisclosure(false);

  return (
    <Card withBorder padding="xs" radius="sm">
      <Group justify="space-between" wrap="nowrap" align="flex-start">
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
            <Text size="xs" c="dimmed" title={formatAbsolute(release.postedAt)}>
              posted {formatRelative(release.postedAt)}
            </Text>
            {release.resolutionPath && (
              <Badge size="xs" variant="dot" color="teal">
                {release.resolutionPath}
              </Badge>
            )}
          </Group>
        </Stack>
        <Group gap={8} wrap="nowrap" align="center">
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
        </Group>
      </Group>

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

  return (
    <Modal
      opened
      onClose={onClose}
      title="Move release to another series"
      size="lg"
      centered
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
