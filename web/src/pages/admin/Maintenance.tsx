import {
  Alert,
  Button,
  Card,
  Group,
  List,
  Modal,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import {
  useInvalidateCoverCache,
  useInvalidateMetadataHashes,
  useRefreshAllSeries,
} from "@/api/mutations";

/// Admin maintenance page. Hosts cross-provider operational actions that
/// don't fit on a per-provider or per-source surface. The page is designed
/// to grow siblings (rebuild FTS, clear negative cache, etc.) without
/// restructuring.
export function AdminMaintenancePage() {
  return (
    <Stack gap="lg">
      <Stack gap={4}>
        <Title order={3}>Maintenance</Title>
        <Text size="sm" c="dimmed">
          Cross-provider operational actions. These are escape hatches for rare
          situations; most day-to-day operations live on the per-source or
          per-provider pages.
        </Text>
      </Stack>
      <InvalidateMetadataHashesCard />
      <RefreshAllSeriesCard />
      <InvalidateCoverCacheCard />
    </Stack>
  );
}

function InvalidateMetadataHashesCard() {
  const [opened, { open, close }] = useDisclosure(false);
  const invalidate = useInvalidateMetadataHashes();

  const handleConfirm = () => {
    invalidate.mutate(
      {},
      {
        onSuccess: (data) => {
          close();
          const invalidated = data?.invalidated ?? 0;
          const skippedManual = data?.skippedManual ?? 0;
          const detail =
            skippedManual > 0
              ? `${invalidated} cleared, ${skippedManual} manual row(s) left alone`
              : `${invalidated} cleared`;
          notifications.show({
            color: invalidated > 0 ? "blue" : "gray",
            title: "Metadata hashes invalidated",
            message: `${detail}. Trigger a series refresh to rewrite the rows.`,
          });
        },
        onError: (e) =>
          notifications.show({
            color: "red",
            title: "Invalidation failed",
            message: (e as Error).message,
          }),
      },
    );
  };

  return (
    <Card
      withBorder
      radius="md"
      p="md"
      data-testid="maintenance-invalidate-card"
    >
      <Stack gap="sm">
        <Stack gap={2}>
          <Title order={4}>Invalidate metadata hashes</Title>
          <Text size="sm" c="dimmed">
            Clear cached metadata hashes for every provider-backed series. The
            next refresh tick rewrites each row from the canonical provider
            metadata instead of short-circuiting on a hash match.
          </Text>
        </Stack>
        <Text size="xs" c="dimmed">
          Use this when:
        </Text>
        <List size="xs" c="dimmed" withPadding>
          <List.Item>
            A new denormalized column was added to the series table and existing
            rows still show the old shape (e.g. NULL volumes or chapters).
          </List.Item>
          <List.Item>
            You suspect the persisted metadata has drifted from what the
            provider currently publishes.
          </List.Item>
        </List>
        <Text size="xs" c="dimmed">
          Manual rows are always left untouched. After clearing, trigger{" "}
          <Text component="span" fw={600}>
            Refresh all series metadata
          </Text>{" "}
          below (or wait for the next series-refresh cron tick) to actually
          rewrite the rows.
        </Text>
        <Group justify="flex-end">
          <Button
            color="orange"
            variant="light"
            size="xs"
            onClick={open}
            data-testid="maintenance-invalidate-button"
          >
            Invalidate metadata hashes
          </Button>
        </Group>
      </Stack>

      <Modal
        opened={opened}
        onClose={close}
        title="Invalidate metadata hashes?"
        centered
      >
        <Stack gap="md">
          <Alert color="orange" variant="light">
            This will clear cached hashes for every provider-backed series. The
            next refresh will rewrite every affected row. Manual rows are left
            alone.
          </Alert>
          <Text size="sm">
            The operation itself is cheap; it's the refresh that follows that
            does the work.
          </Text>
          <Group justify="flex-end" gap="xs">
            <Button
              variant="default"
              size="xs"
              onClick={close}
              disabled={invalidate.isPending}
            >
              Cancel
            </Button>
            <Button
              color="orange"
              size="xs"
              onClick={handleConfirm}
              loading={invalidate.isPending}
              data-testid="maintenance-invalidate-confirm"
            >
              Invalidate
            </Button>
          </Group>
        </Stack>
      </Modal>
    </Card>
  );
}

function InvalidateCoverCacheCard() {
  const [opened, { open, close }] = useDisclosure(false);
  const invalidate = useInvalidateCoverCache();

  const handleConfirm = () => {
    invalidate.mutate(undefined, {
      onSuccess: (data) => {
        close();
        const files = data?.filesDeleted ?? 0;
        const bytes = data?.bytesFreed ?? 0;
        notifications.show({
          color: files > 0 ? "blue" : "gray",
          title: "Cover cache invalidated",
          message:
            files > 0
              ? `${files} file(s) deleted, ${formatBytes(bytes)} freed`
              : "Cache was already empty.",
        });
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Cover cache invalidation failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Card
      withBorder
      radius="md"
      p="md"
      data-testid="maintenance-invalidate-covers-card"
    >
      <Stack gap="sm">
        <Stack gap={2}>
          <Title order={4}>Invalidate cover cache</Title>
          <Text size="sm" c="dimmed">
            Delete every file under the cover-proxy cache directory. Covers are
            re-fetched on demand the next time the UI requests them; in-flight
            browser tabs may continue to show cached bytes until their own cache
            TTL expires.
          </Text>
        </Stack>
        <Text size="xs" c="dimmed">
          Use this when:
        </Text>
        <List size="xs" c="dimmed" withPadding>
          <List.Item>
            An upstream cover was corrected and the proxy is still serving the
            old bytes.
          </List.Item>
          <List.Item>You want to reclaim disk space.</List.Item>
        </List>
        <Group justify="flex-end">
          <Button
            color="orange"
            variant="light"
            size="xs"
            onClick={open}
            data-testid="maintenance-invalidate-covers-button"
          >
            Invalidate cover cache
          </Button>
        </Group>
      </Stack>

      <Modal
        opened={opened}
        onClose={close}
        title="Invalidate cover cache?"
        centered
      >
        <Stack gap="md">
          <Alert color="orange" variant="light">
            This deletes every file under the cover-proxy cache directory.
            Covers come back into existence on the next request, at the cost of
            one upstream fetch per series.
          </Alert>
          <Group justify="flex-end" gap="xs">
            <Button
              variant="default"
              size="xs"
              onClick={close}
              disabled={invalidate.isPending}
            >
              Cancel
            </Button>
            <Button
              color="orange"
              size="xs"
              onClick={handleConfirm}
              loading={invalidate.isPending}
              data-testid="maintenance-invalidate-covers-confirm"
            >
              Invalidate
            </Button>
          </Group>
        </Stack>
      </Modal>
    </Card>
  );
}

// Compact human-readable byte formatter. Used only by the cover-cache
// invalidation notification; keep it local rather than promoting to a
// shared util until a second caller appears.
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function RefreshAllSeriesCard() {
  const refresh = useRefreshAllSeries();

  const handleClick = () => {
    refresh.mutate(undefined, {
      onSuccess: (data) => {
        if (data?.triggered) {
          notifications.show({
            color: "blue",
            title: "Series refresh triggered",
            message: `${data.provider}: up to ${data.batchSize} row(s), min age ${data.minAgeDays}d`,
          });
        } else {
          notifications.show({
            color: "gray",
            title: "Refresh already running",
            message: `${data?.provider ?? "active provider"}: a tick is already in flight`,
          });
        }
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Refresh-all failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Card withBorder radius="md" p="md" data-testid="maintenance-refresh-card">
      <Stack gap="sm">
        <Stack gap={2}>
          <Title order={4}>Refresh all series metadata</Title>
          <Text size="sm" c="dimmed">
            Run a series-metadata refresh tick against the active provider now,
            instead of waiting for the next cron tick. Honors the configured
            batch size and minimum row age; manual rows are skipped. Pair with
            the invalidate card above when adding a denormalized column to the
            series table.
          </Text>
        </Stack>
        <Group justify="flex-end">
          <Button
            size="xs"
            variant="light"
            onClick={handleClick}
            loading={refresh.isPending}
            data-testid="maintenance-refresh-button"
          >
            Refresh all series metadata
          </Button>
        </Group>
      </Stack>
    </Card>
  );
}
