import {
  Anchor,
  Badge,
  Box,
  Button,
  Card,
  Center,
  Group,
  Image,
  Loader,
  Select,
  Stack,
  Text,
  TextInput,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { useEffect, useState } from "react";
import { useLinkRelease } from "@/api/mutations";
import {
  type ProviderSearchHit,
  type SeriesListItem,
  useProviderSearch,
  useProviders,
  useSeriesList,
} from "@/api/queries";
import {
  coverProxyForSeries,
  coverProxyForUrl,
  providerUrl,
} from "@/api/utils";

export const CANDIDATE_PLACEHOLDER =
  "data:image/svg+xml;utf8,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 3 4%22%3E%3Crect width=%223%22 height=%224%22 fill=%22%23ced4da%22/%3E%3C/svg%3E";

/// Compact "N vols · M ch" badge from provider metadata, shown on candidate
/// cards and provider-search hits so the operator can match a release against
/// the series' published length. Renders nothing when neither count is known.
export function MetadataCounts({
  totalVolumes,
  totalChapters,
}: {
  totalVolumes?: number | null;
  totalChapters?: number | null;
}) {
  const parts: string[] = [];
  if (typeof totalVolumes === "number") {
    parts.push(`${totalVolumes} vol${totalVolumes === 1 ? "" : "s"}`);
  }
  if (typeof totalChapters === "number") {
    parts.push(`${totalChapters} ch`);
  }
  if (parts.length === 0) {
    return null;
  }
  return (
    <Badge size="xs" variant="light" color="gray" data-testid="metadata-counts">
      {parts.join(" · ")}
    </Badge>
  );
}

/// Catalog search panel: find a series already in the local catalog
/// (provider-backed or manual) and link/relink the release to it. Closes the
/// gap that the resolver and provider search both miss: a manual series has no
/// external id, so it never surfaces as a candidate or in provider search —
/// but recurring releases of it still need to attach to the same row. Also
/// the path for moving a misclassified release to the right existing series.
export function LinkExistingPanel({
  releaseId,
  seedQuery,
  onLinked,
}: {
  releaseId: string;
  seedQuery: string;
  /// Fired after a successful link, so the host can close its modal. The
  /// success toast is shown here regardless.
  onLinked?: () => void;
}) {
  const link = useLinkRelease();
  const [query, setQuery] = useState(seedQuery);
  const [debounced, setDebounced] = useState(seedQuery);

  useEffect(() => {
    const handle = window.setTimeout(() => setDebounced(query), 300);
    return () => window.clearTimeout(handle);
  }, [query]);

  // Blank query falls back to the most-recent series, which is a sensible
  // default browse for "I just made this one".
  const results = useSeriesList({ q: debounced, pageSize: 20 });
  const items = results.data?.items ?? [];

  const handlePick = (series: SeriesListItem) => {
    link.mutate(
      { releaseId, body: { seriesId: series.id } },
      {
        onSuccess: () => {
          notifications.show({
            color: "green",
            message: `Linked to “${series.canonicalTitle}”`,
          });
          onLinked?.();
        },
        onError: (e) =>
          notifications.show({
            color: "red",
            title: "Link failed",
            message: (e as Error).message,
          }),
      },
    );
  };

  return (
    <Stack gap="md">
      <TextInput
        label="Search the catalog"
        description="Matches every series you've already discovered, including manual ones"
        placeholder="Series title"
        value={query}
        onChange={(e) => setQuery(e.currentTarget.value)}
        data-testid="link-existing-search"
        autoFocus
      />
      <ExistingSeriesResults
        items={items}
        loading={results.isFetching}
        disabled={link.isPending}
        onPick={handlePick}
      />
    </Stack>
  );
}

function ExistingSeriesResults({
  items,
  loading,
  disabled,
  onPick,
}: {
  items: SeriesListItem[];
  loading: boolean;
  disabled: boolean;
  onPick: (series: SeriesListItem) => void;
}) {
  if (loading && items.length === 0) {
    return (
      <Center py="md">
        <Loader size="sm" />
      </Center>
    );
  }
  if (items.length === 0) {
    return (
      <Text size="xs" c="dimmed">
        No series in the catalog match. Try a different title, or create a
        manual series.
      </Text>
    );
  }
  return (
    <Stack gap={6} data-testid="link-existing-results">
      <Text size="xs" fw={500} c="dimmed" tt="uppercase">
        {items.length} match{items.length === 1 ? "" : "es"}
      </Text>
      <Stack gap={6} mah={400} style={{ overflowY: "auto" }}>
        {items.map((s) => (
          <Card
            key={s.id}
            withBorder
            padding="xs"
            radius="sm"
            data-testid={`existing-series-${s.id}`}
            style={{ flexShrink: 0 }}
          >
            <Group justify="space-between" wrap="nowrap" align="center">
              <Group gap="sm" wrap="nowrap" style={{ minWidth: 0, flex: 1 }}>
                <Box w={42} miw={42} h={56}>
                  <Image
                    src={
                      s.coverUrl
                        ? coverProxyForSeries(s.id)
                        : CANDIDATE_PLACEHOLDER
                    }
                    fallbackSrc={CANDIDATE_PLACEHOLDER}
                    alt={s.canonicalTitle}
                    radius="sm"
                    h={56}
                    fit="cover"
                  />
                </Box>
                <Stack gap={2} style={{ minWidth: 0, flex: 1 }}>
                  <Text
                    size="sm"
                    fw={500}
                    lineClamp={1}
                    title={s.canonicalTitle}
                  >
                    {s.canonicalTitle}
                  </Text>
                  <Group gap={6} wrap="wrap">
                    {s.kind && (
                      <Badge size="xs" variant="light" color="indigo">
                        {s.kind}
                      </Badge>
                    )}
                    {typeof s.year === "number" && (
                      <Badge size="xs" variant="light" color="gray">
                        {s.year}
                      </Badge>
                    )}
                    {s.metadataSource === "manual" && (
                      <Badge size="xs" variant="light" color="grape">
                        manual
                      </Badge>
                    )}
                  </Group>
                </Stack>
              </Group>
              <Button
                size="xs"
                variant="light"
                onClick={() => onPick(s)}
                disabled={disabled}
                data-testid={`link-existing-${s.id}`}
              >
                Link
              </Button>
            </Group>
          </Card>
        ))}
      </Stack>
    </Stack>
  );
}

/// Provider search panel. Two paths share one UI:
///
/// - Paste an external ID → exact lookup, single result, one click.
/// - Type a title → debounced search, scrollable result list, click-to-link.
///
/// External ID takes priority when both are filled; the helper text makes
/// that explicit.
export function ProviderSearchPanel({
  releaseId,
  seedQuery,
  onLinked,
}: {
  releaseId: string;
  seedQuery: string;
  onLinked?: () => void;
}) {
  const providers = useProviders();
  const link = useLinkRelease();
  const [provider, setProvider] = useState<string | null>(null);
  const [title, setTitle] = useState(seedQuery);
  const [externalId, setExternalId] = useState("");
  // Debounce the title input so each keystroke doesn't fire a search.
  const [debouncedTitle, setDebouncedTitle] = useState(seedQuery);

  useEffect(() => {
    const handle = window.setTimeout(() => setDebouncedTitle(title), 300);
    return () => window.clearTimeout(handle);
  }, [title]);

  const options =
    providers.data?.items.map((p) => ({
      value: p.id,
      label: p.active ? `${p.displayName} (active)` : p.displayName,
    })) ?? [];

  const activeId =
    providers.data?.items.find((p) => p.active)?.id ??
    options[0]?.value ??
    null;

  const effectiveProvider = provider ?? activeId;

  const search = useProviderSearch({
    providerId: effectiveProvider,
    q: debouncedTitle,
    externalId,
  });

  const handleLink = (chosenExternalId: string, displayLabel: string) => {
    if (!effectiveProvider) return;
    link.mutate(
      {
        releaseId,
        body: { provider: effectiveProvider, externalId: chosenExternalId },
      },
      {
        onSuccess: () => {
          notifications.show({
            color: "green",
            message: `Linked to ${displayLabel}`,
          });
          onLinked?.();
        },
        onError: (e) => {
          notifications.show({
            color: "red",
            title: "Link failed",
            message: (e as Error).message,
          });
        },
      },
    );
  };

  return (
    <Stack gap="md">
      <Select
        label="Provider"
        data={options}
        value={effectiveProvider}
        onChange={setProvider}
        allowDeselect={false}
        searchable={options.length > 5}
      />
      <TextInput
        label="External ID"
        description="Paste a provider ID to look up directly (takes priority over title)"
        placeholder="e.g. 12345"
        value={externalId}
        onChange={(e) => setExternalId(e.currentTarget.value)}
        data-testid="search-external-id"
      />
      <TextInput
        label="Title"
        placeholder="Search by series title"
        value={title}
        onChange={(e) => setTitle(e.currentTarget.value)}
        disabled={externalId.trim().length > 0}
        data-testid="search-title"
      />

      <SearchResults
        provider={effectiveProvider}
        hits={search.data?.hits ?? []}
        loading={search.isFetching}
        enabled={Boolean(
          effectiveProvider && (debouncedTitle.trim() || externalId.trim()),
        )}
        disabled={link.isPending}
        onPick={handleLink}
      />
    </Stack>
  );
}

function SearchResults({
  provider,
  hits,
  loading,
  enabled,
  disabled,
  onPick,
}: {
  provider: string | null;
  hits: ProviderSearchHit[];
  loading: boolean;
  enabled: boolean;
  disabled: boolean;
  onPick: (externalId: string, displayLabel: string) => void;
}) {
  if (!enabled) {
    return (
      <Text size="xs" c="dimmed">
        Enter a title or external ID above to search.
      </Text>
    );
  }
  if (loading && hits.length === 0) {
    return (
      <Center py="md">
        <Loader size="sm" />
      </Center>
    );
  }
  if (hits.length === 0) {
    return (
      <Text size="xs" c="dimmed">
        No results.
      </Text>
    );
  }
  return (
    <Stack gap={6} data-testid="search-results">
      <Text size="xs" fw={500} c="dimmed" tt="uppercase">
        {hits.length} result{hits.length === 1 ? "" : "s"}
      </Text>
      <Stack gap={6} mah={400} style={{ overflowY: "auto" }}>
        {hits.map((h) => (
          <Card
            key={`${h.externalId}-${h.title}`}
            withBorder
            padding="xs"
            radius="sm"
            data-testid={`search-hit-${h.externalId}`}
            style={{ flexShrink: 0 }}
          >
            <Group justify="space-between" wrap="nowrap" align="center">
              <Group gap="sm" wrap="nowrap" style={{ minWidth: 0, flex: 1 }}>
                <Box w={42} miw={42} h={56}>
                  <Image
                    src={
                      h.coverUrl
                        ? coverProxyForUrl(h.coverUrl)
                        : CANDIDATE_PLACEHOLDER
                    }
                    fallbackSrc={CANDIDATE_PLACEHOLDER}
                    alt={h.title}
                    radius="sm"
                    h={56}
                    fit="cover"
                  />
                </Box>
                <Stack gap={2} style={{ minWidth: 0, flex: 1 }}>
                  <Group gap={6} wrap="nowrap" style={{ minWidth: 0 }}>
                    <Text
                      size="sm"
                      fw={500}
                      lineClamp={1}
                      title={h.title}
                      style={{ minWidth: 0, flex: 1 }}
                    >
                      {h.title}
                    </Text>
                    {provider &&
                      (() => {
                        const href = providerUrl(provider, h.externalId);
                        return href ? (
                          <Anchor
                            href={href}
                            target="_blank"
                            rel="noreferrer noopener"
                            size="xs"
                            title="Open on provider"
                          >
                            view ↗
                          </Anchor>
                        ) : null;
                      })()}
                  </Group>
                  {h.nativeTitle && (
                    <Text size="xs" c="dimmed" lineClamp={1}>
                      {h.nativeTitle}
                    </Text>
                  )}
                  <Group gap={6} wrap="wrap">
                    <Badge size="xs" variant="default">
                      score {h.score.toFixed(2)}
                    </Badge>
                    <MetadataCounts
                      totalVolumes={h.totalVolumes}
                      totalChapters={h.totalChapters}
                    />
                    {h.year && (
                      <Badge size="xs" variant="light" color="gray">
                        {h.year}
                      </Badge>
                    )}
                    {h.kind && (
                      <Badge size="xs" variant="light" color="indigo">
                        {h.kind}
                      </Badge>
                    )}
                    {h.status && (
                      <Badge size="xs" variant="light" color="teal">
                        {h.status}
                      </Badge>
                    )}
                  </Group>
                </Stack>
              </Group>
              <Button
                size="xs"
                variant="light"
                onClick={() => onPick(h.externalId, h.title)}
                disabled={disabled}
                data-testid={`link-hit-${h.externalId}`}
              >
                Link
              </Button>
            </Group>
          </Card>
        ))}
      </Stack>
    </Stack>
  );
}
