import {
  Alert,
  Anchor,
  Badge,
  Button,
  Card,
  Center,
  Container,
  Group,
  Loader,
  Paper,
  SimpleGrid,
  Stack,
  Text,
  Title,
  Tooltip,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { Link } from "@tanstack/react-router";
import {
  usePollAllSources,
  usePollSource,
  useRefreshAllProviders,
  useRefreshProvider,
} from "@/api/mutations";
import {
  type ProviderConfigDto,
  type ProviderDto,
  type SourceConfigDto,
  type SourceDto,
  useProviders,
  useSources,
} from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";
import { AdminAuthGate } from "@/components/AdminAuthGate";
import { useAdminAuth } from "@/stores/auth";

export function AdminPage() {
  return (
    <AdminAuthGate>
      <AdminDashboard />
    </AdminAuthGate>
  );
}

function AdminDashboard() {
  const clearToken = useAdminAuth((s) => s.clear);
  return (
    <Container size="xl" py="lg">
      <Stack gap="xl">
        <Group justify="space-between" align="baseline" wrap="wrap">
          <Stack gap={2}>
            <Title order={2}>Admin</Title>
            <Text size="sm" c="dimmed">
              Inspect runtime state and force-trigger scheduler work.
            </Text>
          </Stack>
          <Group gap="sm">
            <Anchor component={Link} to="/review" size="sm">
              Review queue →
            </Anchor>
            <Tooltip label="Forget the admin token in this browser">
              <Button
                variant="subtle"
                size="xs"
                color="gray"
                onClick={() => clearToken()}
              >
                Sign out
              </Button>
            </Tooltip>
          </Group>
        </Group>

        <SourcesSection />
        <ProvidersSection />
      </Stack>
    </Container>
  );
}

function SourcesSection() {
  const sources = useSources();
  const pollAll = usePollAllSources();

  const handlePollAll = () => {
    pollAll.mutate(undefined, {
      onSuccess: (data) => {
        const triggered = data?.results.filter((r) => r.triggered).length ?? 0;
        const skipped = data?.results.filter((r) => r.skipped).length ?? 0;
        notifications.show({
          color: triggered > 0 ? "blue" : "gray",
          message: `${triggered} triggered, ${skipped} already running`,
        });
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Trigger-all failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Stack gap="md">
      <Group justify="space-between" align="baseline" wrap="wrap">
        <Stack gap={2}>
          <Title order={3}>Discovery sources</Title>
          <Text size="sm" c="dimmed">
            {sources.isLoading
              ? "loading…"
              : `${sources.data?.items.length ?? 0} configured`}
          </Text>
        </Stack>
        <Button
          size="xs"
          variant="light"
          onClick={handlePollAll}
          loading={pollAll.isPending}
          disabled={!sources.data?.items.length}
          data-testid="poll-all-sources"
        >
          Trigger all
        </Button>
      </Group>

      {sources.isError && (
        <Alert color="red" title="Failed to load sources">
          {(sources.error as Error)?.message ?? "Unknown error"}
        </Alert>
      )}

      {sources.isLoading && !sources.data && (
        <Center py="lg">
          <Loader />
        </Center>
      )}

      {sources.data && sources.data.items.length === 0 && (
        <Alert color="gray" title="No sources registered">
          Add `[[sources]]` entries in the tsundoku config and restart.
        </Alert>
      )}

      {sources.data && sources.data.items.length > 0 && (
        <SimpleGrid cols={{ base: 1, md: 2 }} spacing="md">
          {sources.data.items.map((src) => (
            <SourceCard key={src.name} source={src} />
          ))}
        </SimpleGrid>
      )}
    </Stack>
  );
}

function SourceCard({ source }: { source: SourceDto }) {
  const poll = usePollSource();
  const busy = poll.isPending;

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
              <Text fw={600}>{source.name}</Text>
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
          <Button
            size="xs"
            variant="light"
            onClick={handlePoll}
            loading={busy}
            disabled={source.config?.enabled === false}
            data-testid={`poll-${source.name}`}
          >
            Trigger
          </Button>
        </Group>
        <SourceConfigBlock config={source.config} />
      </Stack>
    </Paper>
  );
}

function SourceStatusLine({ source }: { source: SourceDto }) {
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

function SourceConfigBlock({ config }: { config?: SourceConfigDto | null }) {
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
          value={
            <Anchor
              href={config.feedUrl}
              size="xs"
              target="_blank"
              rel="noreferrer noopener"
              lineClamp={1}
              title={config.feedUrl}
            >
              {config.feedUrl}
            </Anchor>
          }
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

function ProvidersSection() {
  const providers = useProviders();
  const refreshAll = useRefreshAllProviders();

  const handleRefreshAll = () => {
    refreshAll.mutate(undefined, {
      onSuccess: (data) => {
        const triggered = data?.results.filter((r) => r.triggered).length ?? 0;
        const skipped = data?.results.filter((r) => r.skipped).length ?? 0;
        notifications.show({
          color: triggered > 0 ? "blue" : "gray",
          message: `${triggered} triggered, ${skipped} already running`,
        });
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
    <Stack gap="md">
      <Group justify="space-between" align="baseline" wrap="wrap">
        <Stack gap={2}>
          <Title order={3}>Metadata providers</Title>
          <Text size="sm" c="dimmed">
            {providers.isLoading
              ? "loading…"
              : `${providers.data?.items.length ?? 0} configured`}
          </Text>
        </Stack>
        <Button
          size="xs"
          variant="light"
          onClick={handleRefreshAll}
          loading={refreshAll.isPending}
          disabled={!providers.data?.items.length}
          data-testid="refresh-all-providers"
        >
          Refresh all
        </Button>
      </Group>

      {providers.isError && (
        <Alert color="red" title="Failed to load providers">
          {(providers.error as Error)?.message ?? "Unknown error"}
        </Alert>
      )}

      {providers.isLoading && !providers.data && (
        <Center py="lg">
          <Loader />
        </Center>
      )}

      {providers.data && providers.data.items.length > 0 && (
        <SimpleGrid cols={{ base: 1, md: 2 }} spacing="md">
          {providers.data.items.map((p) => (
            <ProviderCard key={p.id} provider={p} />
          ))}
        </SimpleGrid>
      )}
    </Stack>
  );
}

function ProviderCard({ provider }: { provider: ProviderDto }) {
  const refresh = useRefreshProvider();

  const handleRefresh = () => {
    refresh.mutate(provider.id, {
      onSuccess: (data) => {
        if (data?.skipped) {
          notifications.show({
            color: "gray",
            message: `${provider.id}: refresh already in flight`,
          });
        } else {
          notifications.show({
            color: "blue",
            message: `${provider.id}: refresh triggered`,
          });
        }
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: `${provider.id}: refresh failed`,
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
              <Text fw={600}>{provider.displayName}</Text>
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
          <Button
            size="xs"
            variant="light"
            onClick={handleRefresh}
            loading={refresh.isPending}
            data-testid={`refresh-${provider.id}`}
          >
            Refresh
          </Button>
        </Group>
        <ProviderConfigBlock config={provider.config} />
      </Stack>
    </Paper>
  );
}

function ProviderStatusLine({ provider }: { provider: ProviderDto }) {
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
        refreshed {formatRelative(ref.fetchedAt)}
      </Text>
      {parts.length > 0 && (
        <Text size="xs" c="dimmed">
          • {parts.join(", ")}
        </Text>
      )}
    </Group>
  );
}

function ProviderConfigBlock({
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
            <Anchor
              size="xs"
              href={config.offlineDumpUrl}
              target="_blank"
              rel="noreferrer noopener"
              lineClamp={1}
              title={config.offlineDumpUrl}
            >
              {config.offlineDumpUrl}
            </Anchor>
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

function ConfigRow({
  label,
  value,
  mono,
}: {
  label: string;
  value: React.ReactNode;
  mono?: boolean;
}) {
  return (
    <Group gap="xs" wrap="nowrap" align="baseline">
      <Text
        size="xs"
        c="dimmed"
        ff="monospace"
        miw={120}
        style={{ flexShrink: 0 }}
      >
        {label}
      </Text>
      {typeof value === "string" ? (
        <Text size="xs" ff={mono ? "monospace" : undefined}>
          {value}
        </Text>
      ) : (
        <Card padding={0} radius={0} bg="transparent" style={{ minWidth: 0 }}>
          {value}
        </Card>
      )}
    </Group>
  );
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(value >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}
