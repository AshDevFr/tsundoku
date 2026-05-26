import {
  Alert,
  Badge,
  Center,
  Group,
  Loader,
  Paper,
  Stack,
  Table,
  Text,
  Title,
} from "@mantine/core";
import { useIdMapMetrics } from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";

/// "ID maps" page. Two tables stacked: per-provider counts from
/// `series_external_ids`, then the persisted MangaUpdates redirect
/// cache (modern slugs vs. tombstones).
export function AdminIdMapsPage() {
  const data = useIdMapMetrics();
  if (data.isLoading && !data.data) {
    return (
      <Center py="lg">
        <Loader />
      </Center>
    );
  }
  if (data.isError) {
    return (
      <Alert color="red" title="Failed to load id-map metrics">
        {(data.error as Error)?.message ?? "Unknown error"}
      </Alert>
    );
  }
  return (
    <Stack gap="lg" data-testid="admin-id-maps">
      <Stack gap={4}>
        <Title order={3}>ID maps</Title>
        <Text size="sm" c="dimmed">
          Foreign-id mappings recorded against this catalog. Useful for checking
          how complete the per-provider linkage is.
        </Text>
      </Stack>

      <Paper withBorder radius="md" p="md">
        <Stack gap="sm">
          <Title order={5}>External-id rows by provider</Title>
          {data.data?.externalIds.length ? (
            <Table withRowBorders={false} verticalSpacing={4}>
              <Table.Thead>
                <Table.Tr>
                  <Table.Th>Provider</Table.Th>
                  <Table.Th>Rows</Table.Th>
                </Table.Tr>
              </Table.Thead>
              <Table.Tbody>
                {data.data.externalIds.map((row) => (
                  <Table.Tr
                    key={row.provider}
                    data-testid={`id-map-row-${row.provider}`}
                  >
                    <Table.Td>
                      <Badge size="xs" variant="light" color="indigo">
                        {row.provider}
                      </Badge>
                    </Table.Td>
                    <Table.Td>
                      <Text size="sm" ff="monospace">
                        {row.count.toLocaleString()}
                      </Text>
                    </Table.Td>
                  </Table.Tr>
                ))}
              </Table.Tbody>
            </Table>
          ) : (
            <Text size="sm" c="dimmed">
              No foreign-id mappings recorded yet.
            </Text>
          )}
        </Stack>
      </Paper>

      <Paper withBorder radius="md" p="md">
        <Stack gap="sm">
          <Group justify="space-between" align="baseline" wrap="wrap">
            <Stack gap={2}>
              <Title order={5}>MangaUpdates redirect cache</Title>
              <Text size="xs" c="dimmed">
                Persisted in{" "}
                <Text span ff="monospace">
                  mangaupdates_id_map
                </Text>
                . One row per legacy id we've translated; tombstones mark ids
                MangaUpdates has retired.
              </Text>
            </Stack>
            {typeof data.data?.mangaupdatesRedirectCache.lastResolvedAt ===
              "number" && (
              <Text
                size="xs"
                c="dimmed"
                title={formatAbsolute(
                  data.data.mangaupdatesRedirectCache.lastResolvedAt,
                )}
              >
                last resolved{" "}
                {formatRelative(
                  data.data.mangaupdatesRedirectCache.lastResolvedAt,
                )}
              </Text>
            )}
          </Group>
          <Group gap="md" wrap="wrap">
            <Stack gap={0} miw={64}>
              <Text size="lg" fw={600} lh={1}>
                {(
                  data.data?.mangaupdatesRedirectCache.modernCount ?? 0
                ).toLocaleString()}
              </Text>
              <Text size="xs" c="dimmed" tt="uppercase">
                modern slugs
              </Text>
            </Stack>
            <Stack gap={0} miw={64}>
              <Text size="lg" fw={600} lh={1}>
                {(
                  data.data?.mangaupdatesRedirectCache.tombstoneCount ?? 0
                ).toLocaleString()}
              </Text>
              <Text size="xs" c="dimmed" tt="uppercase">
                tombstones
              </Text>
            </Stack>
          </Group>
        </Stack>
      </Paper>
    </Stack>
  );
}
