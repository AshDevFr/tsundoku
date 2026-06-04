import {
  Alert,
  Badge,
  Button,
  Center,
  Divider,
  Group,
  Loader,
  Paper,
  ScrollArea,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { Link } from "@tanstack/react-router";
import { useTestDownload } from "@/api/mutations";
import {
  type DownloadStatusDto,
  type HealthCheckDto,
  type SendRecordDto,
  useDownloadStatus,
} from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";
import { ConfigRow } from "@/components/admin/atoms";

/// Cap the check/send history lists so a long audit trail scrolls inside the
/// card instead of pushing the page down. Two-line rows, so ~4 fit before it
/// scrolls.
const HISTORY_MAX_HEIGHT = 168;

/// Admin page for the send-to-torrent-client integration: connection info,
/// live reachability with an on-demand test, and the recent check / send
/// history. Full-width single card — there is only ever one configured client.
export function AdminDownloadPage() {
  const status = useDownloadStatus();

  return (
    <Stack gap="md">
      <Stack gap={2}>
        <Title order={3}>Download client</Title>
        <Text size="xs" c="dimmed">
          The torrent client discovered releases are pushed to (ruTorrent in
          v1). Configured from the <code>[download]</code> block; this page is
          read-only plus an on-demand connection test.
        </Text>
      </Stack>

      {status.isError && (
        <Alert color="red" title="Failed to load download status">
          {(status.error as Error)?.message ?? "Unknown error"}
        </Alert>
      )}

      {status.isLoading && !status.data && (
        <Center py="lg">
          <Loader />
        </Center>
      )}

      {status.data && !status.data.enabled && (
        <Alert
          color="gray"
          variant="light"
          title="Integration disabled"
          data-testid="download-disabled"
        >
          Set <code>download.enabled = true</code> (and a <code>base_url</code>)
          in your config to enable the send-to-client action and this page.
        </Alert>
      )}

      {status.data?.enabled && <DownloadCard status={status.data} />}
    </Stack>
  );
}

function DownloadCard({ status }: { status: DownloadStatusDto }) {
  const test = useTestDownload();

  const runTest = () => {
    test.mutate(undefined, {
      onSuccess: (data) => {
        notifications.show({
          color: data?.reachable ? "teal" : "red",
          message: data?.reachable
            ? "Download client reachable"
            : `Unreachable: ${data?.lastError ?? "unknown error"}`,
        });
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Connection test failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Paper withBorder radius="md" p="md" data-testid="download-card">
      <Stack gap="md">
        <Group justify="space-between" align="flex-start" wrap="nowrap">
          <Stack gap={4} style={{ minWidth: 0 }}>
            <Group gap="xs" align="center" wrap="wrap">
              <Text fw={600}>{status.kind ?? "download client"}</Text>
              <ReachableBadge status={status} />
            </Group>
            <HealthLine status={status} />
          </Stack>
          <Button
            size="xs"
            variant="light"
            onClick={runTest}
            loading={test.isPending}
            data-testid="download-test"
          >
            Test connection
          </Button>
        </Group>

        <Stack gap={4}>
          <ConfigRow label="base_url" value={status.baseUrl ?? "—"} mono />
          <ConfigRow label="kind" value={status.kind ?? "—"} mono />
          <ConfigRow
            label="credentials"
            value={
              <Badge
                size="xs"
                variant="light"
                color={status.hasCredentials ? "teal" : "gray"}
              >
                {status.hasCredentials ? "set" : "none"}
              </Badge>
            }
          />
          <ConfigRow label="default_label" value={status.defaultLabel ?? "—"} />
          <ConfigRow
            label="default_start"
            value={status.defaultStart ? "true" : "false"}
            mono
          />
          <ConfigRow
            label="prefer_torrent_file"
            value={status.preferTorrentFile ? "true" : "false"}
            mono
          />
          <ConfigRow
            label="health_cron"
            value={status.healthCron ?? "—"}
            mono
          />
        </Stack>

        {(status.recentChecks.length > 0 || status.recentSends.length > 0) && (
          <Divider />
        )}
        <RecentChecks checks={status.recentChecks} />
        {status.recentChecks.length > 0 && status.recentSends.length > 0 && (
          <Divider />
        )}
        <RecentSends sends={status.recentSends} />
      </Stack>
    </Paper>
  );
}

/// Green/red reachability pill. Gray "never tested" before the first probe.
function ReachableBadge({ status }: { status: DownloadStatusDto }) {
  if (typeof status.lastTestAt !== "number") {
    return (
      <Badge size="sm" color="gray" variant="light">
        never tested
      </Badge>
    );
  }
  return (
    <Badge
      size="sm"
      color={status.reachable ? "green" : "red"}
      variant="light"
      data-testid="download-reachable"
    >
      {status.reachable ? "Reachable" : "Unreachable"}
    </Badge>
  );
}

function HealthLine({ status }: { status: DownloadStatusDto }) {
  if (typeof status.lastTestAt !== "number") {
    return (
      <Text size="xs" c="dimmed">
        no connection test has run yet
      </Text>
    );
  }
  return (
    <Group gap={6} wrap="wrap">
      <Text size="xs" c="dimmed" title={formatAbsolute(status.lastTestAt)}>
        last tested {formatRelative(status.lastTestAt)}
      </Text>
      {!status.reachable && status.lastError && (
        <Text size="xs" c="red" lineClamp={1} title={status.lastError}>
          • {status.lastError}
        </Text>
      )}
    </Group>
  );
}

function RecentChecks({ checks }: { checks: HealthCheckDto[] }) {
  if (checks.length === 0) return null;
  return (
    <Stack gap="xs">
      <Text size="xs" fw={600} c="dimmed" tt="uppercase">
        Recent checks
      </Text>
      <ScrollArea.Autosize
        mah={HISTORY_MAX_HEIGHT}
        type="hover"
        offsetScrollbars="y"
      >
        <Stack gap="sm" pr="sm">
          {checks.map((c) => (
            <Stack key={c.id} gap={2}>
              <Group gap="xs" wrap="nowrap" align="center">
                <Badge
                  size="xs"
                  variant="light"
                  color={c.reachable ? "green" : "red"}
                  style={{ flexShrink: 0 }}
                >
                  {c.reachable ? "up" : "down"}
                </Badge>
                <Text
                  size="xs"
                  fw={500}
                  lineClamp={1}
                  style={{ flex: 1, minWidth: 0 }}
                >
                  {c.reachable ? "Reachable" : "Unreachable"}
                </Text>
                <Text
                  size="xs"
                  c="dimmed"
                  style={{ flexShrink: 0 }}
                  title={formatAbsolute(c.checkedAt)}
                >
                  {formatRelative(c.checkedAt)}
                </Text>
              </Group>
              <Group gap="xs" wrap="wrap" align="baseline" pl={4}>
                <Text size="xs" c="dimmed">
                  via {c.trigger}
                </Text>
                {c.error && (
                  <Text size="xs" c="red" lineClamp={1} title={c.error}>
                    · {c.error}
                  </Text>
                )}
              </Group>
            </Stack>
          ))}
        </Stack>
      </ScrollArea.Autosize>
    </Stack>
  );
}

function RecentSends({ sends }: { sends: SendRecordDto[] }) {
  if (sends.length === 0) return null;
  return (
    <Stack gap="xs">
      <Text size="xs" fw={600} c="dimmed" tt="uppercase">
        Recent sends
      </Text>
      <ScrollArea.Autosize
        mah={HISTORY_MAX_HEIGHT}
        type="hover"
        offsetScrollbars="y"
      >
        <Stack gap="sm" pr="sm">
          {sends.map((s) => (
            <Stack key={s.id} gap={2}>
              <Group gap="xs" wrap="nowrap" align="center">
                <Badge
                  size="xs"
                  variant="light"
                  color={s.success ? "teal" : "red"}
                  style={{ flexShrink: 0 }}
                >
                  {s.success ? "sent" : "failed"}
                </Badge>
                {typeof s.seriesId === "number" ? (
                  // Link directly (not Anchor component={Link}) so TanStack's
                  // typed params infer; the inner Text carries the link color
                  // and clamp.
                  <Link
                    to="/series/$id"
                    params={{ id: String(s.seriesId) }}
                    title={s.releaseTitle ?? s.releaseId}
                    style={{ flex: 1, minWidth: 0, textDecoration: "none" }}
                  >
                    <Text size="xs" fw={500} lineClamp={1} c="blue.4">
                      {s.releaseTitle ?? s.releaseId}
                    </Text>
                  </Link>
                ) : (
                  <Text
                    size="xs"
                    fw={500}
                    lineClamp={1}
                    style={{ flex: 1, minWidth: 0 }}
                    title={s.releaseTitle ?? s.releaseId}
                  >
                    {s.releaseTitle ?? s.releaseId}
                  </Text>
                )}
                <Text
                  size="xs"
                  c="dimmed"
                  style={{ flexShrink: 0 }}
                  title={formatAbsolute(s.sentAt)}
                >
                  {formatRelative(s.sentAt)}
                </Text>
              </Group>
              <Group gap="xs" wrap="wrap" align="baseline" pl={4}>
                <Text size="xs" c="dimmed">
                  via {s.source}
                </Text>
                {s.label && (
                  <Text size="xs" c="dimmed">
                    · label: {s.label}
                  </Text>
                )}
                {s.error && (
                  <Text size="xs" c="red" lineClamp={1} title={s.error}>
                    · {s.error}
                  </Text>
                )}
              </Group>
            </Stack>
          ))}
        </Stack>
      </ScrollArea.Autosize>
    </Stack>
  );
}
