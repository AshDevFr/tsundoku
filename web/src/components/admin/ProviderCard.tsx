import {
  Badge,
  Button,
  Group,
  Paper,
  Stack,
  Text,
  Tooltip,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { Link } from "@tanstack/react-router";
import { useRefreshProvider } from "@/api/mutations";
import type { ProviderConfigDto, ProviderDto } from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";
import { ConfigRow, ExternalLink } from "./atoms";
import { formatBytes } from "./format";
import { JobStatusPill } from "./JobStatusPill";

const LINK_STYLE = { textDecoration: "none", color: "inherit" } as const;

/// Metadata-provider tile shown on the providers list. The trigger label
/// is "Refresh cache" with a tooltip explaining what's actually
/// happening (re-downloading the offline dump). Anchor title navigates
/// into the per-provider detail page.
export function ProviderCard({ provider }: { provider: ProviderDto }) {
  const refresh = useRefreshProvider();

  const handleRefresh = () => {
    refresh.mutate(provider.id, {
      onSuccess: (data) => {
        if (data?.skipped) {
          notifications.show({
            color: "gray",
            message: `${provider.id}: cache refresh already in flight`,
          });
        } else {
          notifications.show({
            color: "blue",
            message: `${provider.id}: cache refresh triggered`,
          });
        }
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: `${provider.id}: cache refresh failed`,
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Paper
      withBorder
      radius="md"
      p="md"
      data-testid={`provider-card-${provider.id}`}
    >
      <Stack gap="sm">
        <Group justify="space-between" align="flex-start" wrap="nowrap">
          <Stack gap={2} style={{ minWidth: 0 }}>
            <Group gap="xs" align="baseline" wrap="wrap">
              <Link
                to="/admin/providers/$id"
                params={{ id: provider.id }}
                style={LINK_STYLE}
              >
                <Text fw={600} component="span" style={{ cursor: "pointer" }}>
                  {provider.displayName}
                </Text>
              </Link>
              <Badge size="xs" color="indigo" variant="light">
                {provider.id}
              </Badge>
              {provider.active && (
                <Badge size="xs" color="teal">
                  active
                </Badge>
              )}
            </Group>
            <ProviderStatusLine provider={provider} />
          </Stack>
          <Group
            gap="xs"
            wrap="nowrap"
            align="center"
            style={{ flexShrink: 0 }}
          >
            <JobStatusPill
              kind="provider"
              id={provider.id}
              inFlight={provider.inFlight}
            />
            <Tooltip
              label="Re-download the offline metadata dump and rebuild the indexes"
              withArrow
            >
              <Button
                size="xs"
                variant="light"
                onClick={handleRefresh}
                loading={refresh.isPending}
                data-testid={`refresh-${provider.id}`}
              >
                Refresh cache
              </Button>
            </Tooltip>
          </Group>
        </Group>
        <ProviderConfigBlock config={provider.config} />
      </Stack>
    </Paper>
  );
}

export function ProviderStatusLine({ provider }: { provider: ProviderDto }) {
  const ref = provider.lastRefresh;
  if (!ref) {
    return (
      <Text size="xs" c="dimmed">
        cache never refreshed
      </Text>
    );
  }
  const parts: string[] = [];
  if (typeof ref.recordCount === "number") {
    parts.push(`${ref.recordCount.toLocaleString()} records`);
  }
  if (typeof ref.bytesDownloaded === "number") {
    parts.push(formatBytes(ref.bytesDownloaded));
  }
  return (
    <Group gap={6} wrap="wrap">
      <Text size="xs" c="dimmed" title={formatAbsolute(ref.fetchedAt)}>
        cache refreshed {formatRelative(ref.fetchedAt)}
      </Text>
      {parts.length > 0 && (
        <Text size="xs" c="dimmed">
          • {parts.join(", ")}
        </Text>
      )}
    </Group>
  );
}

export function ProviderConfigBlock({
  config,
}: {
  config?: ProviderConfigDto | null;
}) {
  if (!config) {
    return (
      <Text size="xs" c="dimmed">
        no typed config block for this provider
      </Text>
    );
  }
  return (
    <Stack gap={4}>
      <ConfigRow
        label="api_key"
        value={
          <Badge
            size="xs"
            color={config.apiKeySet ? "teal" : "gray"}
            variant="light"
            data-testid="api-key-set-badge"
          >
            {config.apiKeySet ? "set" : "not set"}
          </Badge>
        }
      />
      <ConfigRow
        label="api_fallback"
        value={config.apiFallback ? "on" : "off"}
        mono
      />
      <ConfigRow
        label="api_base_url"
        value={
          <Text
            size="xs"
            ff="monospace"
            lineClamp={1}
            title={config.apiBaseUrl}
          >
            {config.apiBaseUrl}
          </Text>
        }
      />
      <ConfigRow
        label="offline_cache"
        value={
          <Badge
            size="xs"
            color={config.offlineCacheLoaded ? "teal" : "gray"}
            variant="light"
          >
            {config.offlineCacheLoaded ? "loaded" : "not loaded"}
          </Badge>
        }
      />
      <ConfigRow
        label="offline_dump_url"
        value={
          config.offlineDumpUrl ? (
            <ExternalLink url={config.offlineDumpUrl} />
          ) : (
            "—"
          )
        }
      />
      <ConfigRow
        label="refresh_cron"
        value={config.offlineRefreshCron ?? "—"}
        mono
      />
      <ConfigRow
        label="negative_ttl"
        value={`${config.negativeCacheTtlDays}d`}
        mono
      />
    </Stack>
  );
}
