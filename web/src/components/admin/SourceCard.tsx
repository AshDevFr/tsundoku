import { Badge, Button, Group, Paper, Stack, Text } from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { Link } from "@tanstack/react-router";
import { usePollSource } from "@/api/mutations";
import type { SourceConfigDto, SourceDto } from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";
import { ConfigRow, ExternalLink } from "./atoms";
import { JobStatusPill } from "./JobStatusPill";

const LINK_STYLE = { textDecoration: "none", color: "inherit" } as const;

/// Full-detail card used on the sources list. The "Open" link takes
/// the operator into the per-source page; "Trigger" runs the manual
/// poll. Both are independent of which page the card lives on.
export function SourceCard({ source }: { source: SourceDto }) {
  const poll = usePollSource();

  const handlePoll = () => {
    poll.mutate(source.name, {
      onSuccess: (data) => {
        if (data?.skipped) {
          notifications.show({
            color: "gray",
            message: `${source.name}: tick already in flight`,
          });
        } else {
          notifications.show({
            color: "blue",
            message: `${source.name}: triggered`,
          });
        }
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: `${source.name}: trigger failed`,
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Paper
      withBorder
      radius="md"
      p="md"
      data-testid={`source-card-${source.name}`}
    >
      <Stack gap="sm">
        <Group justify="space-between" align="flex-start" wrap="nowrap">
          <Stack gap={2} style={{ minWidth: 0 }}>
            <Group gap="xs" align="baseline" wrap="wrap">
              <Link
                to="/admin/sources/$name"
                params={{ name: source.name }}
                style={LINK_STYLE}
              >
                <Text fw={600} component="span" style={{ cursor: "pointer" }}>
                  {source.name}
                </Text>
              </Link>
              <Badge size="xs" color="indigo" variant="light">
                {source.kind}
              </Badge>
              {source.config?.enabled === false && (
                <Badge size="xs" color="gray">
                  disabled
                </Badge>
              )}
            </Group>
            <SourceStatusLine source={source} />
          </Stack>
          <Group gap="xs" wrap="nowrap" align="center">
            <JobStatusPill kind="source" id={source.name} />
            <Button
              size="xs"
              variant="light"
              onClick={handlePoll}
              loading={poll.isPending}
              disabled={source.config?.enabled === false}
              data-testid={`poll-${source.name}`}
            >
              Trigger
            </Button>
          </Group>
        </Group>
        <SourceConfigBlock config={source.config} />
      </Stack>
    </Paper>
  );
}

export function SourceStatusLine({ source }: { source: SourceDto }) {
  if (source.lastError) {
    return (
      <Group gap={6} wrap="wrap">
        <Badge size="xs" color="red" variant="light">
          last error
        </Badge>
        <Text
          size="xs"
          c="red"
          title={
            typeof source.lastPolledAt === "number"
              ? formatAbsolute(source.lastPolledAt)
              : undefined
          }
        >
          {source.lastError}
        </Text>
      </Group>
    );
  }
  if (source.lastPolledAt) {
    return (
      <Group gap={6} wrap="wrap">
        <Text size="xs" c="dimmed" title={formatAbsolute(source.lastPolledAt)}>
          polled {formatRelative(source.lastPolledAt)}
        </Text>
        {source.lastSummary && (
          <Text size="xs" c="dimmed">
            • {source.lastSummary}
          </Text>
        )}
      </Group>
    );
  }
  return (
    <Text size="xs" c="dimmed">
      never polled
    </Text>
  );
}

export function SourceConfigBlock({
  config,
}: {
  config?: SourceConfigDto | null;
}) {
  if (!config) {
    return (
      <Text size="xs" c="dimmed">
        no config block bound (test scaffolding?)
      </Text>
    );
  }
  return (
    <Stack gap={4}>
      <ConfigRow label="cron" value={config.cron ?? "—"} mono />
      {config.feedUrl && (
        <ConfigRow
          label="feed_url"
          value={<ExternalLink url={config.feedUrl} />}
        />
      )}
      <ConfigRow label="timeout" value={`${config.timeoutSeconds}s`} mono />
      <ConfigRow
        label="fetch_details"
        value={config.fetchDetails ? "yes" : "no"}
        mono
      />
    </Stack>
  );
}
