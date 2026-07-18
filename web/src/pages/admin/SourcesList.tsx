import {
  Alert,
  Badge,
  Button,
  Center,
  Group,
  Loader,
  Paper,
  SimpleGrid,
  Stack,
  Table,
  Text,
  Title,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { usePollAllSources } from "@/api/mutations";
import { useSearchEntries, useSources } from "@/api/queries";
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

      <SearchEndpointsSection />
    </Stack>
  );
}

/// Read-only listing of the configured `[[search]]` release-search
/// endpoints. Config-only like sources and the download client: there is
/// deliberately no editing here. Self-gating: renders nothing when no
/// entries are configured (the whole feature is dormant then).
function SearchEndpointsSection() {
  const entries = useSearchEntries();
  const items = entries.data?.items ?? [];
  if (items.length === 0) return null;

  return (
    <Stack gap="xs" data-testid="search-endpoints-section">
      <Stack gap={2}>
        <Title order={4}>Search endpoints</Title>
        <Text size="sm" c="dimmed">
          Targets for the per-series "Search releases" action, from the
          `[[search]]` config. The default entry is the button's primary action.
        </Text>
      </Stack>
      <Paper withBorder radius="md" p="sm" style={{ overflowX: "auto" }}>
        <Table verticalSpacing="xs" fz="sm">
          <Table.Thead>
            <Table.Tr>
              <Table.Th>Name</Table.Th>
              <Table.Th>Kind</Table.Th>
              <Table.Th>Search URL</Table.Th>
              <Table.Th>Max pages</Table.Th>
              <Table.Th>Detail fetch</Table.Th>
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {items.map((e) => (
              <Table.Tr key={e.name} data-testid={`search-endpoint-${e.name}`}>
                <Table.Td>
                  <Group gap={6} wrap="nowrap">
                    <Text size="sm" fw={500}>
                      {e.name}
                    </Text>
                    {e.default && (
                      <Badge size="xs" variant="light" color="blue">
                        default
                      </Badge>
                    )}
                  </Group>
                </Table.Td>
                <Table.Td>{e.kind}</Table.Td>
                <Table.Td>
                  <Text
                    size="sm"
                    ff="monospace"
                    style={{ wordBreak: "break-all" }}
                  >
                    {e.searchUrl}
                  </Text>
                </Table.Td>
                <Table.Td>{e.maxPages}</Table.Td>
                <Table.Td>{e.fetchDetails ? "on" : "off"}</Table.Td>
              </Table.Tr>
            ))}
          </Table.Tbody>
        </Table>
      </Paper>
    </Stack>
  );
}
