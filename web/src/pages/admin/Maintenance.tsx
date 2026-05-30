import {
  Alert,
  Badge,
  Button,
  Card,
  Group,
  List,
  Loader,
  Modal,
  MultiSelect,
  Select,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { useState } from "react";
import {
  useCodexRefresh,
  useInvalidateCoverCache,
  useInvalidateMetadataHashes,
  useRecomputeSpans,
  useReenrichSource,
  useRefreshAllSeries,
} from "@/api/mutations";
import { type CodexStatusDto, useCodexStatus, useSources } from "@/api/queries";
import { formatRelative } from "@/api/utils";

/// Resolution statuses a release can carry, mirrored from the backend's
/// `VALID_STATUSES`. Used to populate the re-enrich status picker.
const REENRICH_STATUS_OPTIONS = [
  "unresolved",
  "ambiguous",
  "review_pending",
  "resolved",
  "standalone",
  "rejected",
];

/// The "needs attention" set, pre-selected in the re-enrich picker. Targets
/// the rows whose details an operator most often wants refreshed, and keeps
/// the default off the large `resolved` / `standalone` buckets so a careless
/// click doesn't kick off thousands of detail-page fetches.
const REENRICH_DEFAULT_STATUSES = ["unresolved", "ambiguous", "review_pending"];

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
      <CodexConnectionCard />
      <InvalidateMetadataHashesCard />
      <RefreshAllSeriesCard />
      <RecomputeSpansCard />
      <ReenrichReleasesCard />
      <InvalidateCoverCacheCard />
    </Stack>
  );
}

/// Mantine color + human label for each Codex `auth_state`. Names the exact
/// operator fix for the two failure modes (wrong key vs under-scoped key).
const AUTH_STATE_META: Record<string, { color: string; label: string }> = {
  ok: { color: "green", label: "Connected" },
  unauthorized: { color: "red", label: "API key rejected (401)" },
  forbidden: { color: "red", label: "API key lacks series:read (403)" },
  unknown: { color: "gray", label: "Not checked yet" },
};

/// Codex integration health + a manual sweep trigger. Reads
/// `GET /codex/status`; the card collapses to a short "disabled" note when the
/// integration is off so it's harmless to always render on the maintenance
/// page.
function CodexConnectionCard() {
  const status = useCodexStatus();
  const refresh = useCodexRefresh();

  const handleRefresh = () => {
    refresh.mutate(undefined, {
      onSuccess: (data) => {
        notifications.show({
          color: data?.triggered ? "blue" : "gray",
          title: data?.triggered
            ? "Codex sync triggered"
            : "Sync already running",
          message: data?.triggered
            ? "A presence sweep is running; ownership badges refresh when it finishes."
            : "A sweep is already in flight; this request was a no-op.",
        });
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Codex refresh failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Card withBorder radius="md" p="md" data-testid="maintenance-codex-card">
      <Stack gap="sm">
        <Stack gap={2}>
          <Title order={4}>Codex integration</Title>
          <Text size="sm" c="dimmed">
            Connection health for the Codex presence sync. Ownership badges on
            the feed come from a periodic sweep; trigger one manually here.
          </Text>
        </Stack>

        {status.isLoading && <Loader size="sm" />}
        {status.isError && (
          <Alert color="red" variant="light">
            Failed to load Codex status: {(status.error as Error)?.message}
          </Alert>
        )}
        {status.data && <CodexStatusBody status={status.data} />}

        {status.data?.enabled && (
          <Group justify="flex-end">
            <Button
              size="xs"
              variant="light"
              onClick={handleRefresh}
              loading={refresh.isPending}
              data-testid="maintenance-codex-refresh-button"
            >
              Refresh now
            </Button>
          </Group>
        )}
      </Stack>
    </Card>
  );
}

function CodexStatusBody({ status }: { status: CodexStatusDto }) {
  if (!status.enabled) {
    return (
      <Alert color="gray" variant="light" data-testid="codex-status-disabled">
        The Codex integration is disabled. Set{" "}
        <code>[codex] enabled = true</code> (with <code>base_url</code> and{" "}
        <code>api_key</code>) to enable it.
      </Alert>
    );
  }
  const auth = AUTH_STATE_META[status.authState] ?? AUTH_STATE_META.unknown;
  return (
    <Stack gap={6} data-testid="codex-status-body">
      <Group gap="xs" wrap="wrap">
        <Badge
          color={status.reachable ? "green" : "red"}
          variant="light"
          data-testid="codex-reachable-badge"
        >
          {status.reachable ? "Reachable" : "Unreachable"}
        </Badge>
        <Badge
          color={auth.color}
          variant="light"
          data-testid="codex-auth-badge"
        >
          {auth.label}
        </Badge>
        {status.codexName && (
          <Text size="sm" c="dimmed">
            {status.codexName}
            {status.codexVersion ? ` v${status.codexVersion}` : ""}
          </Text>
        )}
      </Group>
      <Group gap="lg" wrap="wrap">
        <Text size="sm">
          <Text span fw={600}>
            Linked series:
          </Text>{" "}
          {typeof status.linkedCount === "number" ? status.linkedCount : "—"}
        </Text>
        <Text size="sm">
          <Text span fw={600}>
            Last sync:
          </Text>{" "}
          {typeof status.lastSuccessAt === "number"
            ? formatRelative(status.lastSuccessAt)
            : "never"}
        </Text>
      </Group>
      {status.lastError && (
        <Text size="xs" c="red" data-testid="codex-last-error">
          Last error: {status.lastError}
        </Text>
      )}
    </Stack>
  );
}

/// Re-fetch the detail page for already-persisted releases and refresh their
/// source-derived columns (files, description, extracted links, information
/// link) without touching resolution state. The status picker scopes the
/// walk; pair it with the source picker (a single source today). Use after a
/// parser change adds or fixes a detail-page field on existing rows.
function ReenrichReleasesCard() {
  const sources = useSources();
  const reenrich = useReenrichSource();
  const sourceNames = sources.data?.items.map((s) => s.name) ?? [];
  const [source, setSource] = useState<string | null>(null);
  const [statuses, setStatuses] = useState<string[]>(REENRICH_DEFAULT_STATUSES);
  // Default the picker to the first source once the list loads, without
  // clobbering an explicit choice.
  const effectiveSource = source ?? sourceNames[0] ?? null;

  const handleClick = () => {
    if (!effectiveSource || statuses.length === 0) {
      return;
    }
    reenrich.mutate(
      { name: effectiveSource, statuses },
      {
        onSuccess: (data) => {
          if (data?.triggered) {
            notifications.show({
              color: "blue",
              title: "Re-enrich triggered",
              message: `${data.source}: ${data.statuses.join(", ")}`,
            });
          } else {
            notifications.show({
              color: "gray",
              title: "Source busy",
              message: `${data?.source ?? effectiveSource}: a poll, backfill, or re-enrich is already in flight`,
            });
          }
        },
        onError: (e) =>
          notifications.show({
            color: "red",
            title: "Re-enrich failed",
            message: (e as Error).message,
          }),
      },
    );
  };

  return (
    <Card withBorder radius="md" p="md" data-testid="maintenance-reenrich-card">
      <Stack gap="sm">
        <Stack gap={2}>
          <Title order={4}>Re-enrich release details</Title>
          <Text size="sm" c="dimmed">
            Re-fetch the post detail page for existing releases and refresh
            their files, description, extracted links, and information link.
            Resolution and series links are left untouched. Scope it by status
            below; the default targets the review queue. Re-enriching{" "}
            <Text span fw={600}>
              resolved
            </Text>{" "}
            or{" "}
            <Text span fw={600}>
              standalone
            </Text>{" "}
            can mean many detail-page fetches.
          </Text>
        </Stack>
        <Select
          label="Source"
          data={sourceNames}
          value={effectiveSource}
          onChange={setSource}
          disabled={sources.isLoading || sourceNames.length === 0}
          allowDeselect={false}
          data-testid="reenrich-source-select"
        />
        <MultiSelect
          label="Statuses"
          data={REENRICH_STATUS_OPTIONS}
          value={statuses}
          onChange={setStatuses}
          clearable
          data-testid="reenrich-status-select"
        />
        <Group justify="flex-end">
          <Button
            size="xs"
            variant="light"
            onClick={handleClick}
            loading={reenrich.isPending}
            disabled={!effectiveSource || statuses.length === 0}
            data-testid="maintenance-reenrich-button"
          >
            Re-enrich releases
          </Button>
        </Group>
      </Stack>
    </Card>
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

function RecomputeSpansCard() {
  const recompute = useRecomputeSpans();

  const handleClick = () => {
    recompute.mutate(undefined, {
      onSuccess: (data) => {
        const releases = data?.releasesRewritten ?? 0;
        const series = data?.seriesUpdated ?? 0;
        notifications.show({
          color: releases > 0 || series > 0 ? "blue" : "gray",
          title: "Spans recomputed",
          message:
            releases > 0 || series > 0
              ? `${releases} release span(s) rewritten, ${series} series mark(s) updated`
              : "Everything was already up to date.",
        });
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Recompute failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Card
      withBorder
      radius="md"
      p="md"
      data-testid="maintenance-recompute-card"
    >
      <Stack gap="sm">
        <Stack gap={2}>
          <Title order={4}>Recompute volume / chapter spans</Title>
          <Text size="sm" c="dimmed">
            Re-parse every release's file names (titles as fallback) and rebuild
            each series' "available volumes / chapters" marks from the highest
            number across its linked releases. Makes no network calls.
          </Text>
        </Stack>
        <Text size="xs" c="dimmed">
          Use this when:
        </Text>
        <List size="xs" c="dimmed" withPadding>
          <List.Item>
            The span-parsing logic changed and existing rows should be
            re-evaluated.
          </List.Item>
          <List.Item>
            You're backfilling a catalog whose releases predate span detection
            (the marks show nothing yet).
          </List.Item>
        </List>
        <Text size="xs" c="dimmed">
          Authoritative: a series mark is replaced with the max across its
          linked releases, so values can also go down if an earlier parse
          over-counted.
        </Text>
        <Group justify="flex-end">
          <Button
            size="xs"
            variant="light"
            onClick={handleClick}
            loading={recompute.isPending}
            data-testid="maintenance-recompute-button"
          >
            Recompute spans
          </Button>
        </Group>
      </Stack>
    </Card>
  );
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
