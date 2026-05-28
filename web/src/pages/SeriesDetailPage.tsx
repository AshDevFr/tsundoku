import {
  Alert,
  Anchor,
  AspectRatio,
  Badge,
  Box,
  Button,
  Card,
  Center,
  Container,
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
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import {
  type ReleaseDto,
  useSeriesDetail,
  useSeriesReleases,
} from "@/api/queries";
import { formatAbsolute, formatRelative, providerUrl } from "@/api/utils";
import {
  LinkExistingPanel,
  ProviderSearchPanel,
} from "@/components/ReleaseLinking";
import { seriesDetailRoute } from "@/router";
import { useAdminAuth } from "@/stores/auth";

const COVER_PLACEHOLDER =
  "data:image/svg+xml;utf8,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 3 4%22%3E%3Crect width=%223%22 height=%224%22 fill=%22%23ced4da%22/%3E%3C/svg%3E";

export function SeriesDetailPage() {
  const { id: idStr } = seriesDetailRoute.useParams();
  const id = Number(idStr);
  const detail = useSeriesDetail(Number.isFinite(id) ? id : undefined);
  const releases = useSeriesReleases(Number.isFinite(id) ? id : undefined);

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
              src={s.coverUrl ?? COVER_PLACEHOLDER}
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
                {s.tags.map((t) => (
                  <Badge key={t} size="sm" variant="light" color="blue">
                    {t}
                  </Badge>
                ))}
              </Group>
            )}

            {s.description && (
              <Text size="sm" style={{ whiteSpace: "pre-line" }}>
                {s.description}
              </Text>
            )}

            <Box>
              <Text size="sm" c="dimmed">
                First seen {formatRelative(s.firstSeenAt)} · last release{" "}
                {formatRelative(s.lastReleaseAt)} · metadata {s.metadataSource}{" "}
                ({formatRelative(s.metadataFetchedAt)})
              </Text>
              {(typeof s.highestVolume === "number" ||
                typeof s.highestChapter === "number") && (
                <Text size="sm" c="dimmed">
                  Highest{" "}
                  {typeof s.highestVolume === "number" &&
                    `vol ${s.highestVolume}`}
                  {typeof s.highestVolume === "number" &&
                    typeof s.highestChapter === "number" &&
                    ", "}
                  {typeof s.highestChapter === "number" &&
                    `ch ${s.highestChapter}`}
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
            <Anchor href={release.magnet} size="xs" rel="noreferrer">
              magnet
            </Anchor>
          )}
          {release.torrentUrl && (
            <Anchor
              href={release.torrentUrl}
              size="xs"
              target="_blank"
              rel="noreferrer noopener"
            >
              .torrent
            </Anchor>
          )}
          {release.ddlUrl && (
            <Anchor
              href={release.ddlUrl}
              size="xs"
              target="_blank"
              rel="noreferrer noopener"
            >
              DDL
            </Anchor>
          )}
          {isAdmin && (
            <Tooltip label="Wrong series? Move this release to the correct one.">
              <Button
                size="compact-xs"
                variant="subtle"
                color="gray"
                onClick={openMove}
                data-testid={`move-release-${release.id}`}
              >
                Move
              </Button>
            </Tooltip>
          )}
        </Group>
      </Group>

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
