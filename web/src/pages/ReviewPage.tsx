import {
  ActionIcon,
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
import { useEffect, useMemo, useRef, useState } from "react";
import {
  useBulkLink,
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
  type ReleaseDto,
  type ReviewCandidateDto,
  type UnresolvedRelease,
  useSources,
  useUnresolvedReleases,
} from "@/api/queries";
import {
  coverProxyForSeries,
  formatAbsolute,
  formatRelative,
  providerUrl,
} from "@/api/utils";
import {
  ExtractedLinks,
  InformationLink,
  ReleaseDescription,
  ReleaseFiles,
} from "@/components/ReleaseDetails";
import {
  BulkLinkPanel,
  CANDIDATE_PLACEHOLDER,
  LinkExistingPanel,
  MetadataCounts,
  ProviderSearchPanel,
} from "@/components/ReleaseLinking";
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

/// Ordering options for the queue. Title sorts (case-insensitive on the
/// server) group a series' releases together so they can be selected and
/// linked in bulk; the recency sorts mirror the rest of the release views.
const REVIEW_SORTS = [
  { value: "observed_desc", label: "Newest first" },
  { value: "observed_asc", label: "Oldest first" },
  { value: "title_asc", label: "Title A→Z" },
  { value: "title_desc", label: "Title Z→A" },
  { value: "posted_desc", label: "Posted (newest)" },
  { value: "posted_asc", label: "Posted (oldest)" },
];

export function ReviewPage() {
  const [page, setPage] = useState(1);
  const [searchInput, setSearchInput] = useState("");
  const [debouncedQ, setDebouncedQ] = useState("");
  const [sourceName, setSourceName] = useState<string | null>(null);
  const [format, setFormat] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [sort, setSort] = useState<string | null>(null);
  // Explicit per-card selection. `selectAllMatching` overrides it: the bulk
  // action then targets every release matching the current filters (resolved
  // server-side), not just the checked ids on this page.
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [selectAllMatching, setSelectAllMatching] = useState(false);
  // Anchor for shift+click range selection: the index (into the current
  // page's `pageIds`) of the last release toggled on its own. A shift+click
  // selects every card between this anchor and the clicked card. Reset
  // alongside the selection so it can't point into a stale page ordering.
  const lastSelectedIndex = useRef<number | null>(null);
  // Cards collapse to a one-line header so a sorted run of the same series is
  // easy to scan and bulk-select. Default expanded (an id present here is
  // collapsed); membership is per-id so the expand/collapse-all toggle and
  // the per-card chevron stay in sync.
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [
    confirmRejectOpen,
    { open: openConfirmReject, close: closeConfirmReject },
  ] = useDisclosure(false);
  const [bulkLinkOpen, { open: openBulkLink, close: closeBulkLink }] =
    useDisclosure(false);
  const [bulkCreateOpen, { open: openBulkCreate, close: closeBulkCreate }] =
    useDisclosure(false);

  const resetSelection = () => {
    setSelected(new Set());
    setSelectAllMatching(false);
    lastSelectedIndex.current = null;
  };

  // Debounce the search box so each keystroke doesn't fire a request. A
  // settled query also restarts pagination: a narrower result set could
  // otherwise leave the operator stranded on a now-empty page. The matching
  // set changes too, so any in-flight selection is dropped.
  //
  // Skip the very first run: on mount `searchInput` is already in sync with
  // `debouncedQ`, so the only thing the initial timer would do is fire a
  // selection reset ~300ms in — wiping a selection the operator made in that
  // window. Reset only on an actual edit.
  const searchDebounceArmed = useRef(false);
  useEffect(() => {
    if (!searchDebounceArmed.current) {
      searchDebounceArmed.current = true;
      return;
    }
    const handle = window.setTimeout(() => {
      setDebouncedQ(searchInput);
      setPage(1);
      // Inline the reset (rather than calling resetSelection) so the effect's
      // only dependency stays `searchInput`; the state setters are stable.
      setSelected(new Set());
      setSelectAllMatching(false);
      lastSelectedIndex.current = null;
    }, 300);
    return () => window.clearTimeout(handle);
  }, [searchInput]);

  const queue = useUnresolvedReleases({
    page,
    q: debouncedQ,
    sourceName: sourceName ?? undefined,
    format: format ?? undefined,
    status: status ?? undefined,
    sort: sort ?? undefined,
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
  // Re-ordering changes which rows land on the current page, so reset
  // pagination and drop any selection that referred to the old ordering.
  const changeSort = (v: string | null) => {
    setSort(v);
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

  // Toggle the card at `index` on the current page. A plain click flips that
  // one release and re-anchors the range. A shift+click extends from the
  // anchor to here, selecting the whole run (the anchor itself stays put so
  // successive shift+clicks re-extend from the same origin).
  const toggleAt = (index: number, shiftKey: boolean) => {
    const id = pageIds[index];
    if (id === undefined) return;
    setSelectAllMatching(false);
    if (shiftKey && lastSelectedIndex.current !== null) {
      const start = Math.min(lastSelectedIndex.current, index);
      const end = Math.max(lastSelectedIndex.current, index);
      const rangeIds = pageIds.slice(start, end + 1);
      setSelected((prev) => {
        const next = new Set(prev);
        for (const rid of rangeIds) next.add(rid);
        return next;
      });
      return;
    }
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
    lastSelectedIndex.current = index;
  };
  const toggleAllOnPage = () => {
    setSelectAllMatching(false);
    lastSelectedIndex.current = null;
    setSelected((prev) => {
      if (pageIds.every((id) => prev.has(id))) return new Set();
      return new Set(pageIds);
    });
  };

  const allCollapsed =
    pageIds.length > 0 && pageIds.every((id) => collapsed.has(id));
  const toggleCollapseAll = () => {
    setCollapsed(allCollapsed ? new Set() : new Set(pageIds));
  };
  const toggleCollapseOne = (id: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // Explicit ids for the bulk-link / bulk-create flows. These need a concrete
  // selection (linking a whole "all matching" set to one series is never the
  // intent), so they're gated to the non-`selectAllMatching` case below.
  const selectedIds = useMemo(() => [...selected], [selected]);
  // Seed the bulk search with the first selected release's cleaned query (or
  // title) so the operator usually doesn't have to retype it.
  const bulkSeed = useMemo(() => {
    const first = items.find((i) => selected.has(i.id));
    return first?.searchQueries[0] ?? first?.title ?? "";
  }, [items, selected]);

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
        sort={sort}
        onSort={changeSort}
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
              <Button
                variant="subtle"
                color="gray"
                size="xs"
                onClick={toggleCollapseAll}
                data-testid="toggle-collapse-all"
              >
                {allCollapsed ? "Expand all" : "Collapse all"}
              </Button>
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
                {/* Linking targets one concrete series, so it needs an
                    explicit selection: linking a whole "all matching" set to a
                    single series is never the intent. Disabled (with a hint)
                    while "select all matching" is active. */}
                <Tooltip
                  label={
                    selectAllMatching
                      ? "Linking needs an explicit selection, not “all matching”."
                      : "Link every selected release to one series (search the catalog or a provider)."
                  }
                >
                  <Button
                    size="xs"
                    variant="light"
                    color="cyan"
                    onClick={openBulkLink}
                    disabled={selectAllMatching}
                    data-testid="bulk-link"
                  >
                    Link to series
                  </Button>
                </Tooltip>
                <Tooltip
                  label={
                    selectAllMatching
                      ? "Linking needs an explicit selection, not “all matching”."
                      : "Create a manual series and link every selected release to it."
                  }
                >
                  <Button
                    size="xs"
                    variant="light"
                    color="grape"
                    onClick={openBulkCreate}
                    disabled={selectAllMatching}
                    data-testid="bulk-create-series"
                  >
                    Create series
                  </Button>
                </Tooltip>
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
            {items.map((item, index) => (
              <ReviewCard
                key={item.id}
                item={item}
                selected={selected.has(item.id) || selectAllMatching}
                onToggleSelect={(shiftKey) => toggleAt(index, shiftKey)}
                collapsed={collapsed.has(item.id)}
                onToggleCollapse={() => toggleCollapseOne(item.id)}
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

      {/* Mounted only while open so the search panels don't run in the
          background. Both reset the selection and close on success. */}
      {bulkLinkOpen && (
        <BulkLinkModal
          releaseIds={selectedIds}
          seedQuery={bulkSeed}
          onClose={closeBulkLink}
          onLinked={() => {
            resetSelection();
            closeBulkLink();
          }}
        />
      )}
      {bulkCreateOpen && (
        <BulkCreateSeriesModal
          releaseIds={selectedIds}
          seedTitle={bulkSeed}
          onClose={closeBulkCreate}
          onLinked={() => {
            resetSelection();
            closeBulkCreate();
          }}
        />
      )}

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

/// Bulk "assign to series" modal: a thin wrapper over [`BulkLinkPanel`].
/// Mounted only while open so the catalog/provider searches don't run in the
/// background; the selection count is reflected in the title.
function BulkLinkModal({
  releaseIds,
  seedQuery,
  onClose,
  onLinked,
}: {
  releaseIds: string[];
  seedQuery: string;
  onClose: () => void;
  onLinked: () => void;
}) {
  return (
    <Modal
      opened
      onClose={onClose}
      title={`Link ${releaseIds.length} release${releaseIds.length === 1 ? "" : "s"} to a series`}
      size="lg"
      centered
    >
      <Stack gap="md">
        <BulkLinkPanel
          releaseIds={releaseIds}
          seedQuery={seedQuery}
          onLinked={onLinked}
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

/// Create one manual series and link every selected release to it. The
/// series-creation form mirrors the single-release [`CreateSeriesModal`];
/// on submit it creates the series, then bulk-links the whole selection.
function BulkCreateSeriesModal({
  releaseIds,
  seedTitle,
  onClose,
  onLinked,
}: {
  releaseIds: string[];
  seedTitle: string;
  onClose: () => void;
  onLinked: () => void;
}) {
  const createSeries = useCreateSeries();
  const bulkLink = useBulkLink();
  const [title, setTitle] = useState(seedTitle);
  const [kind, setKind] = useState<string | null>(null);
  const [year, setYear] = useState<number | "">("");

  const busy = createSeries.isPending || bulkLink.isPending;
  const canSubmit = title.trim().length > 0 && !busy;

  const handleCreate = async () => {
    try {
      const created = await createSeries.mutateAsync({
        canonicalTitle: title.trim(),
        kind: kind ?? undefined,
        year: typeof year === "number" ? year : undefined,
      });
      if (!created) throw new Error("series create returned no body");
      const result = await bulkLink.mutateAsync({
        ids: releaseIds,
        seriesId: created.id,
        provider: null,
        externalId: null,
      });
      const n = result?.linked ?? releaseIds.length;
      notifications.show({
        color: "green",
        message: `Created “${created.canonicalTitle}” and linked ${n} release${n === 1 ? "" : "s"}`,
      });
      onLinked();
    } catch (e) {
      notifications.show({
        color: "red",
        title: "Create & link failed",
        message: (e as Error).message,
      });
    }
  };

  return (
    <Modal
      opened
      onClose={onClose}
      title="Create series for selection"
      centered
    >
      <Stack gap="md">
        <Text size="xs" c="dimmed">
          Creates one manual series MangaBaka lacks and links all{" "}
          {releaseIds.length} selected release
          {releaseIds.length === 1 ? "" : "s"} to it. It won’t auto-resolve
          future releases.
        </Text>
        <TextInput
          label="Title"
          required
          value={title}
          onChange={(e) => setTitle(e.currentTarget.value)}
          data-testid="bulk-create-series-title"
        />
        <Select
          label="Kind"
          placeholder="(optional)"
          data={["manga", "manhwa", "manhua", "novel", "other"]}
          value={kind}
          onChange={setKind}
          clearable
          data-testid="bulk-create-series-kind"
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
          data-testid="bulk-create-series-year"
        />
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button
            onClick={handleCreate}
            loading={busy}
            disabled={!canSubmit}
            data-testid="bulk-create-series-submit"
          >
            Create &amp; link {releaseIds.length}
          </Button>
        </Group>
      </Stack>
    </Modal>
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
  sort,
  onSort,
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
  sort: string | null;
  onSort: (v: string | null) => void;
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
      <Select
        label="Sort"
        placeholder="Newest first"
        data={REVIEW_SORTS}
        value={sort}
        onChange={onSort}
        clearable
        style={{ width: 170 }}
        data-testid="review-sort"
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
  collapsed,
  onToggleCollapse,
}: {
  item: UnresolvedRelease;
  selected: boolean;
  onToggleSelect: (shiftKey: boolean) => void;
  collapsed: boolean;
  onToggleCollapse: () => void;
}) {
  const link = useLinkRelease();
  const reject = useRejectRelease();
  const keep = useKeepRelease();
  const retry = useRetryRelease();
  // Holds whether shift was held on the click that drives the next select
  // toggle (see the select Checkbox below).
  const shiftKeyRef = useRef(false);
  const [manualOpen, { open: openManual, close: closeManual }] =
    useDisclosure(false);
  // Seed for the manual-search modal. Comment-suggested links pre-fill it; the
  // plain "Search & link manually" button clears it first.
  const [manualSeed, setManualSeed] = useState<{
    externalId?: string;
    idSource?: string;
  }>({});
  const openManualSeeded = (externalId: string, idSource: string) => {
    setManualSeed({ externalId, idSource });
    openManual();
  };
  const openManualBlank = () => {
    setManualSeed({});
    openManual();
  };
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
            // The `change` event doesn't carry modifier keys, so stash the
            // shift state from the preceding `click` (same gesture, fires
            // first) and read it back when the toggle actually commits.
            onClick={(e) => {
              shiftKeyRef.current = e.shiftKey;
            }}
            onChange={() => {
              onToggleSelect(shiftKeyRef.current);
              shiftKeyRef.current = false;
            }}
            aria-label="Select release"
            data-testid={`select-${item.id}`}
          />
          <Box style={{ flex: 1, minWidth: 0 }}>
            <ReleaseHeader release={item} />
          </Box>
          <ActionIcon
            variant="subtle"
            color="gray"
            onClick={onToggleCollapse}
            aria-label={collapsed ? "Expand release" : "Collapse release"}
            data-testid={`collapse-${item.id}`}
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden="true"
              style={{
                transform: collapsed ? "rotate(-90deg)" : "none",
                transition: "transform 150ms ease",
              }}
            >
              <path d="m6 9 6 6 6-6" />
            </svg>
          </ActionIcon>
        </Group>
        {/* Conditionally rendered (not Mantine <Collapse>) so a collapsed
            card mounts none of its detail panels — the candidate/provider
            searches never run for rows the operator isn't looking at. */}
        {!collapsed && (
          <Stack gap="sm">
            <ExtractedLinks links={item.extractedLinks} />
            <InformationLink url={item.informationUrl} />
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
            {/* Untrusted provider links pulled from the post's comments.
                Offered as one-click seeded lookups the operator confirms —
                never auto-linked. */}
            {item.commentSuggestedLinks &&
              Object.entries(item.commentSuggestedLinks).filter(([, v]) => v)
                .length > 0 && (
                <Box data-testid="comment-suggestions">
                  <Text size="xs" c="dimmed" mb={4}>
                    Suggested in comments (unverified):
                  </Text>
                  <Group gap="xs">
                    {Object.entries(item.commentSuggestedLinks)
                      .filter(([, url]) => url)
                      .map(([provider, url]) => (
                        <Button
                          key={provider}
                          variant="default"
                          size="xs"
                          onClick={() =>
                            openManualSeeded(url as string, provider)
                          }
                          data-testid={`comment-suggestion-${provider}`}
                        >
                          Look up {provider}
                        </Button>
                      ))}
                  </Group>
                </Box>
              )}
            <Group justify="space-between" wrap="wrap" gap="xs">
              <Group gap="xs">
                <Button
                  variant="light"
                  size="xs"
                  onClick={openManualBlank}
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
                  disabled={
                    link.isPending || reject.isPending || keep.isPending
                  }
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
                    disabled={
                      link.isPending || reject.isPending || retry.isPending
                    }
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
        )}
      </Stack>

      <ProviderSearchModal
        opened={manualOpen}
        onClose={closeManual}
        releaseId={item.id}
        seedQuery={item.searchQueries[0] ?? item.title}
        seedExternalId={manualSeed.externalId}
        seedIdSource={manualSeed.idSource}
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
/// releases of it still need to attach to the same row. The catalog search
/// itself is shared with the series-page "Move" flow.
function LinkExistingModal({
  onClose,
  releaseId,
  seedQuery,
}: {
  onClose: () => void;
  releaseId: string;
  seedQuery: string;
}) {
  return (
    <Modal
      opened
      onClose={onClose}
      title="Link to existing series"
      size="lg"
      centered
    >
      <Stack gap="md">
        <LinkExistingPanel
          releaseId={releaseId}
          seedQuery={seedQuery}
          onLinked={onClose}
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
                    src={
                      c.seriesCoverUrl
                        ? coverProxyForSeries(c.seriesId)
                        : CANDIDATE_PLACEHOLDER
                    }
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
                    {c.kind && (
                      <Badge size="xs" variant="light" color="indigo">
                        {c.kind}
                      </Badge>
                    )}
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

/// Modal for linking a review-queue release to a provider series. Thin
/// wrapper over the shared [`ProviderSearchPanel`]; mounted only while open
/// so the panel resets its inputs on each open and never searches in the
/// background.
function ProviderSearchModal({
  opened,
  onClose,
  releaseId,
  seedQuery,
  seedExternalId,
  seedIdSource,
}: {
  opened: boolean;
  onClose: () => void;
  releaseId: string;
  seedQuery: string;
  /** Pre-fill the External ID field (e.g. a comment-suggested link). */
  seedExternalId?: string;
  /** Pre-select the "ID source" (canonical foreign provider id). */
  seedIdSource?: string;
}) {
  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title="Search provider"
      size="lg"
      centered
    >
      {opened && (
        <Stack gap="md">
          <ProviderSearchPanel
            releaseId={releaseId}
            seedQuery={seedQuery}
            seedExternalId={seedExternalId}
            seedIdSource={seedIdSource}
            onLinked={onClose}
          />
          <Group justify="flex-end">
            <Button variant="default" onClick={onClose}>
              Close
            </Button>
          </Group>
        </Stack>
      )}
    </Modal>
  );
}
