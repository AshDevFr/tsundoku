import {
  Badge,
  Button,
  Group,
  NumberInput,
  Paper,
  Popover,
  Stack,
  Text,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { useBackfillSource, usePollSource } from "@/api/mutations";
import type { SourceConfigDto, SourceDto } from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";
import { ConfigRow, ExternalLink } from "./atoms";
import { JobStatusPill } from "./JobStatusPill";

/// Default pages to walk when the operator opens the backfill popover.
/// A handful of pages is the typical useful catch-up; the operator can
/// dial it up to MAX_BACKFILL_PAGES or down to 1.
const DEFAULT_BACKFILL_PAGES = 5;
const MAX_BACKFILL_PAGES = 100;

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
          <Group
            gap="xs"
            wrap="nowrap"
            align="center"
            style={{ flexShrink: 0 }}
          >
            <JobStatusPill
              kind="source"
              id={source.name}
              inFlight={source.inFlight}
            />
            <BackfillButton source={source} />
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

/// Backfill is heavier than a poll (it fans out one detail fetch per new
/// release across N listing pages), so it gets a deliberate two-step
/// affordance rather than a one-click trigger: the button opens a popover
/// that asks for a page count and only fires on the explicit confirm.
function BackfillButton({ source }: { source: SourceDto }) {
  const [opened, setOpened] = useState(false);
  const [pages, setPages] = useState<number>(DEFAULT_BACKFILL_PAGES);
  const backfill = useBackfillSource();
  const disabled = source.config?.enabled === false;

  const run = () => {
    backfill.mutate(
      { name: source.name, pages },
      {
        onSuccess: (data) => {
          setOpened(false);
          if (data?.skipped) {
            notifications.show({
              color: "gray",
              message: `${source.name}: backfill already in flight`,
            });
          } else {
            notifications.show({
              color: "blue",
              message: `${source.name}: backfill started (${data?.pages ?? pages} pages)`,
            });
          }
        },
        onError: (e) => {
          setOpened(false);
          notifications.show({
            color: "red",
            title: `${source.name}: backfill failed`,
            message: (e as Error).message,
          });
        },
      },
    );
  };

  return (
    <Popover
      opened={opened}
      onChange={setOpened}
      position="bottom-end"
      withArrow
      shadow="md"
      trapFocus
    >
      <Popover.Target>
        <Button
          size="xs"
          variant="default"
          onClick={() => setOpened((o) => !o)}
          disabled={disabled}
          data-testid={`backfill-${source.name}`}
        >
          Backfill
        </Button>
      </Popover.Target>
      <Popover.Dropdown>
        <Stack gap="xs" w={230}>
          <Text size="sm" fw={600}>
            Backfill {source.name}
          </Text>
          <Text size="xs" c="dimmed">
            Walks older listing pages and resolves every new release. Runs in
            the background; re-running skips rows already stored.
          </Text>
          <NumberInput
            label="Pages to walk"
            min={1}
            max={MAX_BACKFILL_PAGES}
            clampBehavior="strict"
            allowDecimal={false}
            value={pages}
            onChange={(v) =>
              setPages(typeof v === "number" ? v : DEFAULT_BACKFILL_PAGES)
            }
            data-testid={`backfill-pages-${source.name}`}
          />
          <Group justify="flex-end" gap="xs">
            <Button
              size="xs"
              variant="subtle"
              color="gray"
              onClick={() => setOpened(false)}
            >
              Cancel
            </Button>
            <Button
              size="xs"
              onClick={run}
              loading={backfill.isPending}
              data-testid={`backfill-confirm-${source.name}`}
            >
              Run backfill
            </Button>
          </Group>
        </Stack>
      </Popover.Dropdown>
    </Popover>
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
      <ConfigRow label="max_pages" value={String(config.maxPages)} mono />
    </Stack>
  );
}
