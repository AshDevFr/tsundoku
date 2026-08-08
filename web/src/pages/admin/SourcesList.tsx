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
  Switch,
  Table,
  Text,
  TextInput,
  Title,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { useImportRelease, usePollAllSources } from "@/api/mutations";
import {
  useGlobalSearchRuns,
  useSearchEntries,
  useSources,
} from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";
import { SourceCard } from "@/components/admin/SourceCard";
import { useUiPrefs } from "@/stores/uiPrefs";

/// Sources list page. Identical surface to the old admin tab but now
/// addressable at `/admin/sources` and with each card titled as a link
/// into the per-source detail page.
export function AdminSourcesListPage() {
  const sources = useSources();
  const pollAll = usePollAllSources();
  const sourceCardDetails = useUiPrefs((s) => s.sourceCardDetails);
  const toggleSourceCardDetails = useUiPrefs((s) => s.toggleSourceCardDetails);

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
      <Stack gap={2}>
        <Title order={3}>Discovery sources</Title>
        <Text size="sm" c="dimmed">
          The feeds tsundoku polls, plus the search endpoints it can pull a
          single post from.
        </Text>
      </Stack>

      {/* First section: a frequent, deliberate action that depends on nothing
          below it. The source grid runs to twenty-odd cards on a real
          deployment, so anything under it costs a full scroll. */}
      <ImportReleaseCard />

      {/* The count and "Trigger all" describe the grid, so they head it rather
          than floating next to the page title. */}
      <Group justify="space-between" align="baseline" wrap="wrap">
        <Stack gap={2}>
          <Title order={4}>Configured sources</Title>
          <Text size="sm" c="dimmed">
            {sources.isLoading
              ? "loading…"
              : `${sources.data?.items.length ?? 0} configured`}
          </Text>
        </Stack>
        <Group gap="xs" wrap="nowrap">
          {/* One switch for every card rather than a control per card:
              twenty-odd individual toggles to shorten the page would be worse
              than the scroll. Persisted, so it stays how you left it. */}
          <Switch
            size="xs"
            label="Details"
            checked={sourceCardDetails}
            onChange={toggleSourceCardDetails}
            data-testid="toggle-source-details"
          />
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
            <SourceCard
              key={src.name}
              source={src}
              showDetails={sourceCardDetails}
            />
          ))}
        </SimpleGrid>
      )}

      <SearchEndpointsSection />
      <GlobalRecentSearches />
    </Stack>
  );
}

const RESOLUTION_COLOR: Record<string, string> = {
  resolved: "green",
  ambiguous: "yellow",
  review_pending: "yellow",
  unresolved: "orange",
  rejected: "red",
  standalone: "gray",
};

/// Add one release by pasting its post URL, for something the polled feeds
/// never surfaced (an older post, or one outside the configured filters).
/// The URL goes to whichever `[[search]]` entry recognizes it and then
/// through the normal resolve path, so the outcome can be any of the usual
/// statuses — including a trip to the review queue.
///
/// Shares the search endpoints' self-gating: with no `[[search]]` entries
/// there is nothing that could handle a URL, so the card stays hidden.
function ImportReleaseCard() {
  const entries = useSearchEntries();
  const importRelease = useImportRelease();
  const [url, setUrl] = useState("");

  if ((entries.data?.items.length ?? 0) === 0) return null;

  const trimmed = url.trim();
  const result = importRelease.data;

  const submit = () => {
    if (!trimmed) return;
    importRelease.mutate(trimmed, { onSuccess: () => setUrl("") });
  };

  return (
    <Stack gap="xs" data-testid="import-release-card">
      <Stack gap={2}>
        <Title order={4}>Add a release by link</Title>
        <Text size="sm" c="dimmed">
          Paste a post URL from a configured search endpoint. It is fetched,
          stored, and resolved exactly like a polled release — unmatched ones
          land in the review queue.
        </Text>
      </Stack>
      <Paper withBorder radius="md" p="sm">
        <Stack gap="sm">
          <Group gap="xs" align="flex-end" wrap="nowrap">
            <TextInput
              flex={1}
              label="Post URL"
              placeholder="https://nyaa.si/view/1234567"
              value={url}
              onChange={(e) => setUrl(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && submit()}
              data-testid="import-release-url"
            />
            <Button
              onClick={submit}
              loading={importRelease.isPending}
              disabled={!trimmed}
              data-testid="import-release-submit"
            >
              Add
            </Button>
          </Group>

          {importRelease.isError && (
            <Alert
              color="red"
              title="Import failed"
              data-testid="import-release-error"
            >
              {(importRelease.error as Error).message}
            </Alert>
          )}

          {result && (
            <Alert
              color={result.alreadyKnown ? "gray" : "green"}
              title={result.alreadyKnown ? "Already in the catalog" : "Added"}
              data-testid="import-release-result"
            >
              <Group gap="xs" wrap="wrap">
                <Text size="sm">{result.release.title}</Text>
                <Badge
                  size="xs"
                  variant="light"
                  color={
                    RESOLUTION_COLOR[result.release.resolutionStatus] ?? "gray"
                  }
                >
                  {result.release.resolutionStatus}
                </Badge>
                {result.release.seriesId != null && (
                  <Link
                    to="/series/$id"
                    params={{ id: String(result.release.seriesId) }}
                    style={{ textDecoration: "none" }}
                  >
                    <Text size="sm" c="blue.4">
                      view series
                    </Text>
                  </Link>
                )}
              </Group>
            </Alert>
          )}
        </Stack>
      </Paper>
    </Stack>
  );
}

const SEARCH_OUTCOME_META: Record<string, { color: string; label: string }> = {
  success: { color: "green", label: "success" },
  error: { color: "red", label: "failed" },
  running: { color: "blue", label: "running…" },
};

/// Global timeline of per-series release searches across every series,
/// newest first, each row linking back to its series. Hidden until at
/// least one search has run.
function GlobalRecentSearches() {
  const runs = useGlobalSearchRuns();
  const items = runs.data?.items ?? [];
  if (items.length === 0) return null;

  return (
    <Stack gap="xs" data-testid="recent-searches-section">
      <Stack gap={2}>
        <Title order={4}>Recent searches</Title>
        <Text size="sm" c="dimmed">
          Latest per-series release searches across the catalog.
        </Text>
      </Stack>
      <Paper withBorder radius="md" p="sm" style={{ overflowX: "auto" }}>
        <Table verticalSpacing="xs" fz="sm">
          <Table.Thead>
            <Table.Tr>
              <Table.Th>When</Table.Th>
              <Table.Th>Series</Table.Th>
              <Table.Th>Entry</Table.Th>
              <Table.Th>Trigger</Table.Th>
              <Table.Th>Outcome</Table.Th>
              <Table.Th>New</Table.Th>
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {items.map((r) => {
              const meta =
                SEARCH_OUTCOME_META[r.outcome] ?? SEARCH_OUTCOME_META.error;
              return (
                <Table.Tr key={r.id} data-testid={`recent-search-${r.id}`}>
                  <Table.Td>
                    <Text size="sm" c="dimmed" title={formatAbsolute(r.ranAt)}>
                      {formatRelative(r.ranAt)}
                    </Text>
                  </Table.Td>
                  <Table.Td>
                    {/* Link directly (not Anchor component={Link}) so
                        TanStack's typed params infer. */}
                    <Link
                      to="/series/$id"
                      params={{ id: String(r.seriesId) }}
                      style={{ textDecoration: "none" }}
                    >
                      <Text size="sm" c="blue.4" lineClamp={1}>
                        {r.seriesTitle}
                      </Text>
                    </Link>
                  </Table.Td>
                  <Table.Td>{r.searchName}</Table.Td>
                  <Table.Td>{r.trigger}</Table.Td>
                  <Table.Td>
                    <Badge size="xs" variant="light" color={meta.color}>
                      {meta.label}
                    </Badge>
                  </Table.Td>
                  <Table.Td>
                    {r.outcome === "success" ? (r.releasesNew ?? 0) : "—"}
                  </Table.Td>
                </Table.Tr>
              );
            })}
          </Table.Tbody>
        </Table>
      </Paper>
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
