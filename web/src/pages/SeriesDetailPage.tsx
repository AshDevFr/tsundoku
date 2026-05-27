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
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { Link } from "@tanstack/react-router";
import {
  type ReleaseDto,
  useSeriesDetail,
  useSeriesReleases,
} from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";
import { seriesDetailRoute } from "@/router";

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
              <Text c="dimmed" size="sm" lineClamp={2}>
                {s.alternateTitles.join(" · ")}
              </Text>
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
                {s.externalIds.map((x) => (
                  <Badge
                    key={`${x.provider}-${x.externalId}`}
                    variant="dot"
                    color="blue"
                    component={x.externalUrl ? "a" : undefined}
                    {...(x.externalUrl
                      ? {
                          href: x.externalUrl,
                          target: "_blank",
                          rel: "noreferrer noopener",
                          style: { cursor: "pointer", textDecoration: "none" },
                        }
                      : {})}
                  >
                    {x.provider}: {x.externalId}
                  </Badge>
                ))}
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
        <Group gap={4}>
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
        </Group>
      </Group>
    </Card>
  );
}
