import {
  Alert,
  Anchor,
  Badge,
  Box,
  Button,
  Card,
  Center,
  Checkbox,
  Group,
  Image,
  Loader,
  Modal,
  NumberInput,
  Pagination,
  Paper,
  Select,
  Stack,
  Text,
  TextInput,
  Title,
  Tooltip,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { useEffect, useState } from "react";
import {
  useBulkReject,
  useBulkRetry,
  useCreateSeries,
  useKeepRelease,
  useLinkRelease,
  useRejectRelease,
  useRetryAllReleases,
  useRetryRelease,
} from "@/api/mutations";
import {
  type ProviderSearchHit,
  type ReleaseDto,
  type ReviewCandidateDto,
  type SeriesListItem,
  type UnresolvedRelease,
  useProviderSearch,
  useProviders,
  useSeriesList,
  useSources,
  useUnresolvedReleases,
} from "@/api/queries";
import { formatAbsolute, formatRelative, providerUrl } from "@/api/utils";
import {
  ExtractedLinks,
  ReleaseDescription,
  ReleaseFiles,
} from "@/components/ReleaseDetails";
import type { components } from "@/types/api.generated";

type BulkReviewRequest = components["schemas"]["BulkReviewRequest"];

/// Canonical file formats the detector can tag a release with. Hardcoded
/// rather than derived from the queue so the dropdown is stable; an option
/// with no current matches simply yields an empty result.
const REVIEW_FORMATS = [
  "cbz",
  "cbr",
  "cb7",
  "cbt",
  "zip",
  "rar",
  "7z",
  "tar",
  "epub",
  "pdf",
  "mobi",
  "azw3",
];

const REVIEW_STATUSES = [
  { value: "unresolved", label: "Unresolved" },
  { value: "ambiguous", label: "Ambiguous" },
  { value: "review_pending", label: "Review pending" },
];

export function ReviewPage() {
  const [page, setPage] = useState(1);
  const [searchInput, setSearchInput] = useState("");
  const [debouncedQ, setDebouncedQ] = useState("");
  const [sourceName, setSourceName] = useState<string | null>(null);
  const [format, setFormat] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  // Explicit per-card selection. `selectAllMatching` overrides it: the bulk
  // action then targets every release matching the current filters (resolved
  // server-side), not just the checked ids on this page.
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [selectAllMatching, setSelectAllMatching] = useState(false);
  const [
    confirmRejectOpen,
    { open: openConfirmReject, close: closeConfirmReject },
  ] = useDisclosure(false);

  const resetSelection = () => {
    setSelected(new Set());
    setSelectAllMatching(false);
  };

  // Debounce the search box so each keystroke doesn't fire a request. A
  // settled query also restarts pagination: a narrower result set could
  // otherwise leave the operator stranded on a now-empty page. The matching
  // set changes too, so any in-flight selection is dropped.
  useEffect(() => {
    const handle = window.setTimeout(() => {
      setDebouncedQ(searchInput);
      setPage(1);
      // Inline the reset (rather than calling resetSelection) so the effect's
      // only dependency stays `searchInput`; the state setters are stable.
      setSelected(new Set());
      setSelectAllMatching(false);
    }, 300);
    return () => window.clearTimeout(handle);
  }, [searchInput]);

  const queue = useUnresolvedReleases({
    page,
    q: debouncedQ,
    sourceName: sourceName ?? undefined,
    format: format ?? undefined,
    status: status ?? undefined,
  });
  const retryAll = useRetryAllReleases();
  const bulkRetry = useBulkRetry();
  const bulkReject = useBulkReject();

  const hasFilters = Boolean(
    debouncedQ.trim() || sourceName || format || status,
  );
  // The select filters reset pagination + selection synchronously on change
  // (the search box does so via its debounce effect above).
  const changeSource = (v: string | null) => {
    setSourceName(v);
    setPage(1);
    resetSelection();
  };
  const changeFormat = (v: string | null) => {
    setFormat(v);
    setPage(1);
    resetSelection();
  };
  const changeStatus = (v: string | null) => {
    setStatus(v);
    setPage(1);
    resetSelection();
  };
  const clearFilters = () => {
    setSearchInput("");
    setDebouncedQ("");
    setSourceName(null);
    setFormat(null);
    setStatus(null);
    setPage(1);
    resetSelection();
  };
  // Paging is a different view of the same filtered set; drop the per-page
  // selection so a stale checkbox can't ride along to the next page.
  const changePage = (p: number) => {
    setPage(p);
    resetSelection();
  };

  const total = queue.data?.total ?? 0;
  const pageSize = queue.data?.pageSize ?? 20;
  const totalPages = Math.max(1, Math.ceil(total / pageSize));

  const items = queue.data?.items ?? [];
  const pageIds = items.map((i) => i.id);
  const allPageSelected =
    pageIds.length > 0 && pageIds.every((id) => selected.has(id));
  const somePageSelected = pageIds.some((id) => selected.has(id));
  // More matching releases exist than this page shows: offer "select all".
  const moreMatchThanPage = total > items.length;
  const selectionCount = selectAllMatching ? total : selected.size;

  const toggleOne = (id: string) => {
    setSelectAllMatching(false);
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };
  const toggleAllOnPage = () => {
    setSelectAllMatching(false);
    setSelected((prev) => {
      if (pageIds.every((id) => prev.has(id))) return new Set();
      return new Set(pageIds);
    });
  };

  const bulkBody = (): BulkReviewRequest =>
    selectAllMatching
      ? {
          ids: [],
          q: debouncedQ.trim() || null,
          sourceName,
          format,
          status,
        }
      : {
          ids: [...selected],
          q: null,
          sourceName: null,
          format: null,
          status: null,
        };

  const handleBulkRetry = () => {
    bulkRetry.mutate(bulkBody(), {
      onSuccess: (data) => {
        notifications.show({
          color: data?.skipped ? "gray" : "blue",
          message: data?.skipped
            ? "Retry already in progress"
            : `Re-running resolver on ${data?.matched ?? 0} release${data?.matched === 1 ? "" : "s"}`,
        });
        resetSelection();
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Bulk retry failed",
          message: (e as Error).message,
        }),
    });
  };

  const handleBulkReject = () => {
    bulkReject.mutate(bulkBody(), {
      onSuccess: (data) => {
        notifications.show({
          color: "gray",
          message: `Rejected ${data?.rejected ?? 0} release${data?.rejected === 1 ? "" : "s"}`,
        });
        resetSelection();
        closeConfirmReject();
      },
      onError: (e) => {
        notifications.show({
          color: "red",
          title: "Bulk reject failed",
          message: (e as Error).message,
        });
        closeConfirmReject();
      },
    });
  };

  const handleRetryAll = () => {
    retryAll.mutate(undefined, {
      onSuccess: (data) => {
        if (data?.skipped) {
          notifications.show({
            color: "gray",
            message: "Retry already in progress",
          });
        } else {
          notifications.show({
            color: "blue",
            message: "Re-running resolver on the review queue",
          });
        }
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Retry all failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Stack gap="md">
      <Group justify="space-between" align="baseline" wrap="wrap">
        <Stack gap={2}>
          <Title order={3}>Review queue</Title>
          <Text size="sm" c="dimmed">
            {queue.isLoading
              ? "loading…"
              : `${total.toLocaleString()} release${total === 1 ? "" : "s"} awaiting a decision`}
          </Text>
        </Stack>
        <Tooltip label="Re-run the resolver against every release currently in this queue">
          <Button
            variant="light"
            size="xs"
            onClick={handleRetryAll}
            loading={retryAll.isPending}
            disabled={total === 0}
            data-testid="retry-all-button"
          >
            Retry all
          </Button>
        </Tooltip>
      </Group>

      <ReviewFilterBar
        search={searchInput}
        onSearch={setSearchInput}
        sourceName={sourceName}
        onSourceName={changeSource}
        format={format}
        onFormat={changeFormat}
        status={status}
        onStatus={changeStatus}
        hasFilters={hasFilters}
        onClear={clearFilters}
      />

      {queue.isError && (
        <Alert color="red" title="Failed to load review queue">
          {(queue.error as Error)?.message ?? "Unknown error"}
        </Alert>
      )}

      {queue.isLoading && !queue.data && (
        <Center py="xl">
          <Loader />
        </Center>
      )}

      {queue.data && queue.data.items.length === 0 && hasFilters && (
        <Alert color="blue" title="No matches">
          No releases match the current filters.{" "}
          <Anchor component="button" type="button" onClick={clearFilters}>
            Clear filters
          </Anchor>{" "}
          to see the whole queue.
        </Alert>
      )}

      {queue.data && queue.data.items.length === 0 && !hasFilters && (
        <Alert color="green" title="Inbox zero">
          Nothing waiting for review. New unresolved releases will land here as
          the scheduler runs.
        </Alert>
      )}

      {queue.data && queue.data.items.length > 0 && (
        <>
          <Group
            justify="space-between"
            align="center"
            wrap="wrap"
            gap="sm"
            data-testid="review-selection-bar"
          >
            <Group gap="sm" align="center">
              <Checkbox
                size="sm"
                checked={allPageSelected}
                indeterminate={somePageSelected && !allPageSelected}
                onChange={toggleAllOnPage}
                label="Select page"
                data-testid="select-all-page"
              />
              {/* When more matching releases exist than this page shows,
                  offer to act on the whole filtered set. */}
              {allPageSelected &&
                moreMatchThanPage &&
                (selectAllMatching ? (
                  <Text size="sm" data-testid="select-all-matching-active">
                    All {total.toLocaleString()} matching selected.{" "}
                    <Anchor
                      component="button"
                      type="button"
                      onClick={resetSelection}
                    >
                      Clear
                    </Anchor>
                  </Text>
                ) : (
                  <Anchor
                    component="button"
                    type="button"
                    size="sm"
                    onClick={() => setSelectAllMatching(true)}
                    data-testid="select-all-matching"
                  >
                    Select all {total.toLocaleString()} matching
                  </Anchor>
                ))}
            </Group>
            {selectionCount > 0 && (
              <Group gap="xs" data-testid="bulk-action-bar">
                <Text size="sm" fw={500}>
                  {selectionCount.toLocaleString()} selected
                </Text>
                <Button
                  size="xs"
                  variant="light"
                  onClick={handleBulkRetry}
                  loading={bulkRetry.isPending}
                  disabled={bulkReject.isPending}
                  data-testid="bulk-retry"
                >
                  Retry
                </Button>
                <Button
                  size="xs"
                  variant="light"
                  color="red"
                  onClick={openConfirmReject}
                  disabled={bulkRetry.isPending || bulkReject.isPending}
                  data-testid="bulk-reject"
                >
                  Reject
                </Button>
                <Button
                  size="xs"
                  variant="subtle"
                  color="gray"
                  onClick={resetSelection}
                  data-testid="bulk-clear"
                >
                  Clear
                </Button>
              </Group>
            )}
          </Group>

          <Stack gap="md">
            {items.map((item) => (
              <ReviewCard
                key={item.id}
                item={item}
                selected={selected.has(item.id) || selectAllMatching}
                onToggleSelect={() => toggleOne(item.id)}
              />
            ))}
          </Stack>
        </>
      )}

      <Modal
        opened={confirmRejectOpen}
        onClose={closeConfirmReject}
        title="Reject releases"
        centered
      >
        <Stack gap="md">
          <Text size="sm">
            Reject {selectionCount.toLocaleString()} release
            {selectionCount === 1 ? "" : "s"}? They're marked rejected and
            removed from the queue; the resolver leaves them alone afterward.
          </Text>
          <Group justify="flex-end" gap="xs">
            <Button variant="default" onClick={closeConfirmReject}>
              Cancel
            </Button>
            <Button
              color="red"
              onClick={handleBulkReject}
              loading={bulkReject.isPending}
              data-testid="confirm-bulk-reject"
            >
              Reject {selectionCount.toLocaleString()}
            </Button>
          </Group>
        </Stack>
      </Modal>

      {totalPages > 1 && (
        <Center>
          <Pagination
            value={page}
            onChange={changePage}
            total={totalPages}
            size="sm"
          />
        </Center>
      )}
    </Stack>
  );
}

/// Filter controls for the review queue: free-text title search, source
/// instance, file format, and queue status. Source options come from the
/// configured discovery sources; format options are the canonical detector
/// set. Controlled by the parent so the query and pagination react to changes.
function ReviewFilterBar({
  search,
  onSearch,
  sourceName,
  onSourceName,
  format,
  onFormat,
  status,
  onStatus,
  hasFilters,
  onClear,
}: {
  search: string;
  onSearch: (v: string) => void;
  sourceName: string | null;
  onSourceName: (v: string | null) => void;
  format: string | null;
  onFormat: (v: string | null) => void;
  status: string | null;
  onStatus: (v: string | null) => void;
  hasFilters: boolean;
  onClear: () => void;
}) {
  const sources = useSources();
  const sourceOptions =
    sources.data?.items.map((s) => ({ value: s.name, label: s.name })) ?? [];

  return (
    <Group
      gap="sm"
      wrap="wrap"
      align="flex-end"
      data-testid="review-filter-bar"
    >
      <TextInput
        label="Search"
        placeholder="Title contains…"
        value={search}
        onChange={(e) => onSearch(e.currentTarget.value)}
        style={{ flex: "1 1 220px", minWidth: 180 }}
        data-testid="review-search"
      />
      <Select
        label="Source"
        placeholder="Any source"
        data={sourceOptions}
        value={sourceName}
        onChange={onSourceName}
        clearable
        searchable={sourceOptions.length > 5}
        style={{ width: 200 }}
        data-testid="review-source-filter"
      />
      <Select
        label="Format"
        placeholder="Any format"
        data={REVIEW_FORMATS}
        value={format}
        onChange={onFormat}
        clearable
        style={{ width: 140 }}
        data-testid="review-format-filter"
      />
      <Select
        label="Status"
        placeholder="Any status"
        data={REVIEW_STATUSES}
        value={status}
        onChange={onStatus}
        clearable
        style={{ width: 170 }}
        data-testid="review-status-filter"
      />
      {hasFilters && (
        <Button
          variant="subtle"
          color="gray"
          size="sm"
          onClick={onClear}
          data-testid="review-clear-filters"
        >
          Clear
        </Button>
      )}
    </Group>
  );
}

function ReviewCard({
  item,
  selected,
  onToggleSelect,
}: {
  item: UnresolvedRelease;
  selected: boolean;
  onToggleSelect: () => void;
}) {
  const link = useLinkRelease();
  const reject = useRejectRelease();
  const keep = useKeepRelease();
  const retry = useRetryRelease();
  const [manualOpen, { open: openManual, close: closeManual }] =
    useDisclosure(false);
  const [createOpen, { open: openCreate, close: closeCreate }] =
    useDisclosure(false);
  const [
    linkExistingOpen,
    { open: openLinkExisting, close: closeLinkExisting },
  ] = useDisclosure(false);

  const busy =
    link.isPending || reject.isPending || keep.isPending || retry.isPending;

  const handleLinkCandidate = (candidate: ReviewCandidateDto) => {
    link.mutate(
      { releaseId: item.id, body: { seriesId: candidate.seriesId } },
      {
        onSuccess: () => {
          notifications.show({
            color: "green",
            message: `Linked to "${candidate.seriesTitle}"`,
          });
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

  const handleReject = () => {
    reject.mutate(item.id, {
      onSuccess: () =>
        notifications.show({ color: "gray", message: "Release rejected" }),
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Reject failed",
          message: (e as Error).message,
        }),
    });
  };

  const handleKeep = () => {
    keep.mutate(item.id, {
      onSuccess: () =>
        notifications.show({ color: "teal", message: "Kept as standalone" }),
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Keep failed",
          message: (e as Error).message,
        }),
    });
  };

  const handleRetry = () => {
    retry.mutate(item.id, {
      onSuccess: () =>
        notifications.show({ color: "blue", message: "Re-running resolver" }),
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Retry failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Paper withBorder radius="md" p="md" data-testid={`review-card-${item.id}`}>
      <Stack gap="sm">
        <Group align="flex-start" wrap="nowrap" gap="sm">
          <Checkbox
            mt={4}
            checked={selected}
            onChange={onToggleSelect}
            aria-label="Select release"
            data-testid={`select-${item.id}`}
          />
          <Box style={{ flex: 1, minWidth: 0 }}>
            <ReleaseHeader release={item} />
          </Box>
        </Group>
        <ExtractedLinks links={item.extractedLinks} />
        <ReleaseDescription body={item.descriptionHtml} />
        <CleanupTrail
          queries={item.searchQueries}
          rules={item.cleanupRulesApplied}
        />
        <ReleaseFiles files={item.files} />
        <CandidateList
          candidates={item.candidates}
          disabled={busy}
          onPick={handleLinkCandidate}
        />
        <Group justify="space-between" wrap="wrap" gap="xs">
          <Group gap="xs">
            <Button
              variant="light"
              size="xs"
              onClick={openManual}
              disabled={busy}
            >
              Search provider
            </Button>
            <Tooltip label="Link this release to a series already in the catalog (including manual ones).">
              <Button
                variant="light"
                color="cyan"
                size="xs"
                onClick={openLinkExisting}
                disabled={busy}
              >
                Link existing
              </Button>
            </Tooltip>
            <Tooltip label="Create a manual series for something MangaBaka lacks, then link this release to it.">
              <Button
                variant="light"
                color="grape"
                size="xs"
                onClick={openCreate}
                disabled={busy}
              >
                Create series
              </Button>
            </Tooltip>
          </Group>
          <Group gap="xs">
            <Button
              variant="subtle"
              color="gray"
              size="xs"
              onClick={handleRetry}
              loading={retry.isPending}
              disabled={link.isPending || reject.isPending || keep.isPending}
            >
              Retry
            </Button>
            <Tooltip label="Keep as a standalone item (a guidebook, artbook, one-shot) — not a tracked series. Stays in the Kept list.">
              <Button
                variant="subtle"
                color="teal"
                size="xs"
                onClick={handleKeep}
                loading={keep.isPending}
                disabled={link.isPending || reject.isPending || retry.isPending}
              >
                Keep
              </Button>
            </Tooltip>
            <Button
              variant="subtle"
              color="red"
              size="xs"
              onClick={handleReject}
              loading={reject.isPending}
              disabled={link.isPending || retry.isPending || keep.isPending}
            >
              Reject
            </Button>
          </Group>
        </Group>
      </Stack>

      <ProviderSearchModal
        opened={manualOpen}
        onClose={closeManual}
        releaseId={item.id}
        seedQuery={item.searchQueries[0] ?? item.title}
      />
      <CreateSeriesModal
        opened={createOpen}
        onClose={closeCreate}
        releaseId={item.id}
        seedTitle={item.searchQueries[0] ?? item.title}
      />
      {/* Mounted only while open so the catalog search doesn't run in the
          background for every card in the queue. */}
      {linkExistingOpen && (
        <LinkExistingModal
          onClose={closeLinkExisting}
          releaseId={item.id}
          seedQuery={item.searchQueries[0] ?? item.title}
        />
      )}
    </Paper>
  );
}

/// Link the current release to a series that already exists in the local
/// catalog (provider-backed or manual). Closes the gap that the resolver
/// and provider search both miss: a manual series has no external id, so it
/// never surfaces as a candidate or in provider search — but recurring
/// releases of it still need to attach to the same row.
function LinkExistingModal({
  onClose,
  releaseId,
  seedQuery,
}: {
  onClose: () => void;
  releaseId: string;
  seedQuery: string;
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
          onClose();
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
    <Modal
      opened
      onClose={onClose}
      title="Link to existing series"
      size="lg"
      centered
    >
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
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Close
          </Button>
        </Group>
      </Stack>
    </Modal>
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
                    src={s.coverUrl ?? CANDIDATE_PLACEHOLDER}
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

/// Create a provider-less "manual" series and link the current release to
/// it in one step. For real series MangaBaka lacks; the resulting series is
/// flagged `manual` in browse and never auto-resolves future releases.
function CreateSeriesModal({
  opened,
  onClose,
  releaseId,
  seedTitle,
}: {
  opened: boolean;
  onClose: () => void;
  releaseId: string;
  seedTitle: string;
}) {
  const createSeries = useCreateSeries();
  const link = useLinkRelease();
  const [title, setTitle] = useState(seedTitle);
  const [kind, setKind] = useState<string | null>(null);
  const [year, setYear] = useState<number | "">("");

  useEffect(() => {
    if (opened) {
      setTitle(seedTitle);
      setKind(null);
      setYear("");
    }
  }, [opened, seedTitle]);

  const busy = createSeries.isPending || link.isPending;
  const canSubmit = title.trim().length > 0 && !busy;

  const handleCreate = async () => {
    try {
      const created = await createSeries.mutateAsync({
        canonicalTitle: title.trim(),
        kind: kind ?? undefined,
        year: typeof year === "number" ? year : undefined,
      });
      if (!created) throw new Error("series create returned no body");
      await link.mutateAsync({
        releaseId,
        body: { seriesId: created.id },
      });
      notifications.show({
        color: "green",
        message: `Created “${created.canonicalTitle}” and linked the release`,
      });
      onClose();
    } catch (e) {
      notifications.show({
        color: "red",
        title: "Create series failed",
        message: (e as Error).message,
      });
    }
  };

  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title="Create manual series"
      centered
    >
      <Stack gap="md">
        <Text size="xs" c="dimmed">
          For a real series MangaBaka doesn’t have. It won’t auto-resolve future
          releases, so you’ll link those by hand too.
        </Text>
        <TextInput
          label="Title"
          required
          value={title}
          onChange={(e) => setTitle(e.currentTarget.value)}
          data-testid="create-series-title"
        />
        <Select
          label="Kind"
          placeholder="(optional)"
          data={["manga", "manhwa", "manhua", "novel", "other"]}
          value={kind}
          onChange={setKind}
          clearable
          data-testid="create-series-kind"
        />
        <NumberInput
          label="Year"
          placeholder="(optional)"
          value={year}
          onChange={(v) =>
            setYear(typeof v === "number" ? v : v === "" ? "" : Number(v) || "")
          }
          min={1900}
          max={2999}
          allowDecimal={false}
          data-testid="create-series-year"
        />
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button
            onClick={handleCreate}
            loading={busy}
            disabled={!canSubmit}
            data-testid="create-series-submit"
          >
            Create &amp; link
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}

/// Diagnostic strip: shows the cleaned primary search query (with any
/// alternates as small chips) and the rule names that fired during
/// cleanup. Surfaces "what surgery happened" without expanding to a
/// debug pane.
function CleanupTrail({
  queries,
  rules,
}: {
  queries: string[];
  rules: string[];
}) {
  if (queries.length === 0 && rules.length === 0) {
    return null;
  }
  return (
    <Stack gap={4} data-testid="cleanup-trail">
      {queries.length > 0 && (
        <Group gap={6} wrap="wrap" align="center">
          <Text size="xs" c="dimmed" tt="uppercase" fw={500}>
            {queries.length > 1 ? "searched (any)" : "searched"}
          </Text>
          {/* Every query is searched independently; the resolver keeps
              the best match across all of them. Show them all so the
              operator can see exactly what was tried. */}
          {queries.map((q) => (
            <Text key={q} size="xs" ff="monospace">
              “{q}”
            </Text>
          ))}
        </Group>
      )}
      {rules.length > 0 && (
        <Group gap={4} wrap="wrap">
          {rules.map((r) => (
            <Badge
              key={r}
              size="xs"
              variant="outline"
              color="grape"
              ff="monospace"
            >
              {r}
            </Badge>
          ))}
        </Group>
      )}
    </Stack>
  );
}

function ReleaseHeader({ release }: { release: ReleaseDto }) {
  return (
    <Stack gap={4}>
      <Group justify="space-between" align="flex-start" wrap="nowrap">
        <Stack gap={2} style={{ flex: 1, minWidth: 0 }}>
          <Anchor
            href={release.link}
            target="_blank"
            rel="noreferrer noopener"
            size="md"
            fw={600}
            lineClamp={2}
            title={release.title}
          >
            {release.title}
          </Anchor>
          <Group gap={6} wrap="wrap">
            <Badge size="xs" color="indigo" variant="light">
              {release.sourceKind}:{release.sourceName}
            </Badge>
            {release.formats.map((f) => (
              <Badge key={f} size="xs" variant="outline">
                {f}
              </Badge>
            ))}
            <Badge size="xs" color="orange" variant="light">
              {release.resolutionStatus}
            </Badge>
            <Text size="xs" c="dimmed" title={formatAbsolute(release.postedAt)}>
              posted {formatRelative(release.postedAt)}
            </Text>
            {release.resolutionAttempts > 0 && (
              <Text size="xs" c="dimmed">
                {release.resolutionAttempts} attempt
                {release.resolutionAttempts === 1 ? "" : "s"}
              </Text>
            )}
          </Group>
        </Stack>
        <Group gap={6} wrap="nowrap">
          {release.magnet && (
            <Anchor href={release.magnet} size="xs" rel="noreferrer">
              magnet
            </Anchor>
          )}
          {release.torrentUrl && (
            <Anchor
              href={release.torrentUrl}
              size="xs"
              target="_blank"
              rel="noreferrer noopener"
            >
              .torrent
            </Anchor>
          )}
        </Group>
      </Group>
    </Stack>
  );
}

const CANDIDATE_PLACEHOLDER =
  "data:image/svg+xml;utf8,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 3 4%22%3E%3Crect width=%223%22 height=%224%22 fill=%22%23ced4da%22/%3E%3C/svg%3E";

/// Compact "N vols · M ch" badge from provider metadata, shown on candidate
/// cards and provider-search hits so the operator can match a release against
/// the series' published length. Renders nothing when neither count is known.
function MetadataCounts({
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

function CandidateList({
  candidates,
  disabled,
  onPick,
}: {
  candidates: ReviewCandidateDto[];
  disabled: boolean;
  onPick: (c: ReviewCandidateDto) => void;
}) {
  if (candidates.length === 0) {
    return (
      <Text size="sm" c="dimmed">
        No candidate matches — link manually or reject.
      </Text>
    );
  }
  return (
    <Stack gap={6}>
      <Text size="xs" fw={500} c="dimmed" tt="uppercase">
        Candidates
      </Text>
      <Stack gap={6}>
        {candidates.map((c) => (
          <Card
            key={c.seriesId}
            withBorder
            padding="sm"
            radius="sm"
            data-testid={`candidate-${c.seriesId}`}
          >
            <Group justify="space-between" wrap="nowrap" align="flex-start">
              <Group
                gap="sm"
                wrap="nowrap"
                align="flex-start"
                style={{ minWidth: 0, flex: 1 }}
              >
                <Box w={56} miw={56} h={76}>
                  <Image
                    src={c.seriesCoverUrl ?? CANDIDATE_PLACEHOLDER}
                    fallbackSrc={CANDIDATE_PLACEHOLDER}
                    alt={c.seriesTitle}
                    radius="sm"
                    h={76}
                    fit="cover"
                  />
                </Box>
                <Stack gap={4} style={{ minWidth: 0, flex: 1 }}>
                  <Group gap={6} wrap="nowrap" style={{ minWidth: 0 }}>
                    <Text
                      size="md"
                      fw={600}
                      lineClamp={2}
                      title={c.seriesTitle}
                      style={{ minWidth: 0, flex: 1 }}
                    >
                      {c.seriesTitle}
                    </Text>
                    {c.provider &&
                      c.externalId &&
                      (() => {
                        const href = providerUrl(c.provider, c.externalId);
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
                  {c.alternateTitles.length > 0 && (
                    <Text
                      size="xs"
                      c="dimmed"
                      style={{ wordBreak: "break-word" }}
                    >
                      {c.alternateTitles.join(" / ")}
                    </Text>
                  )}
                  <Group gap={6}>
                    <Badge size="xs" variant="default">
                      score {c.score.toFixed(2)}
                    </Badge>
                    <MetadataCounts
                      totalVolumes={c.totalVolumes}
                      totalChapters={c.totalChapters}
                    />
                    {c.reason && (
                      <Text size="xs" c="dimmed">
                        {c.reason}
                      </Text>
                    )}
                  </Group>
                </Stack>
              </Group>
              <Button
                size="xs"
                variant="light"
                onClick={() => onPick(c)}
                disabled={disabled}
                data-testid={`link-candidate-${c.seriesId}`}
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

/// Modal for linking a review-queue release to a provider series. Two
/// paths share one UI:
///
/// - Paste an external ID → exact lookup, single result, one click.
/// - Type a title → debounced search, scrollable result list,
///   click-to-link.
///
/// External ID takes priority when both are filled; the helper text
/// makes that explicit.
function ProviderSearchModal({
  opened,
  onClose,
  releaseId,
  seedQuery,
}: {
  opened: boolean;
  onClose: () => void;
  releaseId: string;
  seedQuery: string;
}) {
  const providers = useProviders();
  const link = useLinkRelease();
  const [provider, setProvider] = useState<string | null>(null);
  const [title, setTitle] = useState(seedQuery);
  const [externalId, setExternalId] = useState("");
  // Debounce the title input so each keystroke doesn't fire a search.
  const [debouncedTitle, setDebouncedTitle] = useState(seedQuery);

  // Reset state when the modal opens against a new release.
  useEffect(() => {
    if (opened) {
      setTitle(seedQuery);
      setDebouncedTitle(seedQuery);
      setExternalId("");
      setProvider(null);
    }
  }, [opened, seedQuery]);

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
    enabled: opened,
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
          onClose();
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
    <Modal
      opened={opened}
      onClose={onClose}
      title="Search provider"
      size="lg"
      centered
    >
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

        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Close
          </Button>
        </Group>
      </Stack>
    </Modal>
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
                    src={h.coverUrl ?? CANDIDATE_PLACEHOLDER}
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
