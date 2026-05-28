import {
  Alert,
  Anchor,
  Badge,
  Button,
  Center,
  Group,
  Loader,
  Pagination,
  Paper,
  Stack,
  Text,
  Title,
  Tooltip,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { useState } from "react";
import { useRetryRelease } from "@/api/mutations";
import { type ReleaseDto, useKeptReleases } from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";
import { formatBytes } from "@/components/admin/format";
import {
  ExtractedLinks,
  ReleaseDescription,
  ReleaseFiles,
} from "@/components/ReleaseDetails";

/// Browse view for releases the operator marked `standalone`: worthwhile
/// one-shots (guidebooks, artbooks) that are deliberately not tracked as a
/// series. These are out of the review queue and never re-resolved unless
/// pulled back in via "Re-resolve".
export function KeptPage() {
  const [page, setPage] = useState(1);
  const kept = useKeptReleases(page);

  const total = kept.data?.total ?? 0;
  const pageSize = kept.data?.pageSize ?? 20;
  const totalPages = Math.max(1, Math.ceil(total / pageSize));

  return (
    <Stack gap="md">
      <Stack gap={2}>
        <Title order={3}>Kept</Title>
        <Text size="sm" c="dimmed">
          {kept.isLoading
            ? "loading…"
            : `${total.toLocaleString()} standalone release${total === 1 ? "" : "s"} (one-shots kept on purpose, not tracked as series)`}
        </Text>
      </Stack>

      {kept.isError && (
        <Alert color="red" title="Failed to load kept releases">
          {(kept.error as Error)?.message ?? "Unknown error"}
        </Alert>
      )}

      {kept.isLoading && !kept.data && (
        <Center py="xl">
          <Loader />
        </Center>
      )}

      {kept.data && kept.data.items.length === 0 && (
        <Alert color="gray" title="Nothing kept yet">
          Releases you mark “Keep” in the review queue land here. Use it for
          things worth holding onto that aren’t a series: guidebooks, artbooks,
          one-shots.
        </Alert>
      )}

      {kept.data && kept.data.items.length > 0 && (
        <Stack gap="sm">
          {kept.data.items.map((release) => (
            <KeptCard key={release.id} release={release} />
          ))}
        </Stack>
      )}

      {totalPages > 1 && (
        <Center>
          <Pagination
            value={page}
            onChange={setPage}
            total={totalPages}
            size="sm"
          />
        </Center>
      )}
    </Stack>
  );
}

function KeptCard({ release }: { release: ReleaseDto }) {
  const retry = useRetryRelease();

  const handleReResolve = () => {
    retry.mutate(release.id, {
      onSuccess: () =>
        notifications.show({
          color: "blue",
          message: "Re-running resolver; moved back into the pipeline",
        }),
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Re-resolve failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Paper
      withBorder
      radius="md"
      p="md"
      data-testid={`kept-card-${release.id}`}
    >
      <Group justify="space-between" align="flex-start" wrap="nowrap" gap="md">
        <Stack gap={4} style={{ flex: 1, minWidth: 0 }}>
          <Anchor
            href={release.link}
            target="_blank"
            rel="noreferrer noopener"
            size="md"
            fw={600}
            lineClamp={2}
            title={release.title}
          >
            {release.title}
          </Anchor>
          <Group gap={6} wrap="wrap">
            <Badge size="xs" color="indigo" variant="light">
              {release.sourceKind}:{release.sourceName}
            </Badge>
            {release.formats.map((f) => (
              <Badge key={f} size="xs" variant="outline">
                {f}
              </Badge>
            ))}
            {typeof release.sizeBytes === "number" && (
              <Text size="xs" c="dimmed">
                {formatBytes(release.sizeBytes)}
              </Text>
            )}
            <Text size="xs" c="dimmed" title={formatAbsolute(release.postedAt)}>
              posted {formatRelative(release.postedAt)}
            </Text>
          </Group>
          <Group gap="sm" wrap="wrap">
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
                download
              </Anchor>
            )}
          </Group>
          <ExtractedLinks links={release.extractedLinks} />
          <ReleaseDescription body={release.descriptionHtml} />
          <ReleaseFiles files={release.files} />
        </Stack>
        <Tooltip label="Send this release back through the resolver (e.g. after a provider refresh).">
          <Button
            variant="subtle"
            color="gray"
            size="xs"
            onClick={handleReResolve}
            loading={retry.isPending}
            data-testid={`re-resolve-${release.id}`}
          >
            Re-resolve
          </Button>
        </Tooltip>
      </Group>
    </Paper>
  );
}
