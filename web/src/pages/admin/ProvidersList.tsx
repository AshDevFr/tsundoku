import {
  Alert,
  Button,
  Center,
  Group,
  Loader,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { useRefreshAllProviders } from "@/api/mutations";
import { useProviders } from "@/api/queries";
import { ProviderCard } from "@/components/admin/ProviderCard";

/// Providers list page. Mirrors SourcesList. The fan-out trigger says
/// "Refresh all caches" so operators know it's re-downloading the
/// offline dumps, not just bumping a state flag.
export function AdminProvidersListPage() {
  const providers = useProviders();
  const refreshAll = useRefreshAllProviders();

  const handleRefreshAll = () => {
    refreshAll.mutate(undefined, {
      onSuccess: (data) => {
        const triggered = data?.results.filter((r) => r.triggered).length ?? 0;
        const skipped = data?.results.filter((r) => r.skipped).length ?? 0;
        notifications.show({
          color: triggered > 0 ? "blue" : "gray",
          message: `${triggered} cache refresh(es) triggered, ${skipped} already running`,
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
          <Text size="xs" c="dimmed">
            Each provider has an offline cache (the published dump). "Refresh
            cache" re-downloads it and rebuilds the indexes.
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
          Refresh all caches
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
        <Stack gap="md">
          {providers.data.items.map((p) => (
            <ProviderCard key={p.id} provider={p} />
          ))}
        </Stack>
      )}
    </Stack>
  );
}
