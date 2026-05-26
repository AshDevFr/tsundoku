import {
  Alert,
  Button,
  Center,
  Group,
  Loader,
  SimpleGrid,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { usePollAllSources } from "@/api/mutations";
import { useSources } from "@/api/queries";
import { SourceCard } from "@/components/admin/SourceCard";

/// Sources list page. Identical surface to the old admin tab but now
/// addressable at `/admin/sources` and with each card titled as a link
/// into the per-source detail page.
export function AdminSourcesListPage() {
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
