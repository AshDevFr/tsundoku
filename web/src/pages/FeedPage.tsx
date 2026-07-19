import {
  Alert,
  Badge,
  Box,
  Button,
  Center,
  Container,
  Drawer,
  Flex,
  Group,
  Loader,
  Pagination,
  SegmentedControl,
  Select,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { useNavigate } from "@tanstack/react-router";
import type { MouseEvent } from "react";
import { useSeriesList } from "@/api/queries";
import { FilterPanel } from "@/components/FilterPanel";
import { SeriesCard } from "@/components/SeriesCard";
import { SeriesListRow } from "@/components/SeriesListRow";
import { SeriesSelectionBar } from "@/components/SeriesSelectionBar";
import { useSeriesSelection } from "@/hooks/useSeriesSelection";
import { feedRoute } from "@/router";
import { useAdminAuth } from "@/stores/auth";
import { countActiveFilters, type FilterSearch } from "@/stores/filters";
import {
  DEFAULT_PAGE_SIZE,
  PAGE_SIZE_OPTIONS,
  useUiPrefs,
} from "@/stores/uiPrefs";

export function FeedPage() {
  const search = feedRoute.useSearch();
  const navigate = useNavigate({ from: feedRoute.fullPath });

  const setSearch = (next: FilterSearch) =>
    navigate({ search: () => next, replace: false });

  // Display preferences (page size, wide layout) are persisted per-device,
  // not encoded in the shareable URL.
  const pageSize = useUiPrefs((s) => s.pageSize);
  const setPageSize = useUiPrefs((s) => s.setPageSize);
  const wideMode = useUiPrefs((s) => s.wideMode);
  const toggleWideMode = useUiPrefs((s) => s.toggleWideMode);
  const view = useUiPrefs((s) => s.view);
  const setView = useUiPrefs((s) => s.setView);
  // On mobile the filter panel collapses into a left drawer so the results
  // grid gets the full width.
  const [filtersOpen, { open: openFilters, close: closeFilters }] =
    useDisclosure(false);
  const activeFilterCount = countActiveFilters(search);

  const list = useSeriesList({ ...search, pageSize });
  const total = list.data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  // Only show Codex badges once a sweep has succeeded (admin-only signal);
  // absent for non-admins, so badges never leak to the public read tier.
  const codexSynced = Boolean(list.data?.codexSyncedAt);

  // Bulk selection (admin-only: every bulk action is an admin write, so
  // non-admins never see the checkboxes). The hook itself drops the
  // selection whenever the visible id set changes (filters, sort, page).
  const isAdmin = useAdminAuth((s) => Boolean(s.token));
  const items = list.data?.items ?? [];
  const pageIds = items.map((i) => i.id);
  const selection = useSeriesSelection(pageIds);
  const selectionFor = (index: number, id: number) =>
    isAdmin
      ? {
          selected: selection.selected.has(id),
          active: selection.selected.size > 0,
          onToggle: (e: MouseEvent) => selection.toggleAt(index, e.shiftKey),
        }
      : undefined;

  return (
    <Container size={wideMode ? "100%" : "xl"} py="lg">
      <Flex
        gap="lg"
        align="flex-start"
        direction={{ base: "column", sm: "row" }}
      >
        <Box
          w={280}
          // Cap the sticky sidebar at the viewport height below the header
          // (56px header + 16px gap above, 16px breathing room below) and
          // scroll it internally; otherwise a panel taller than the screen
          // pins its bottom controls permanently offscreen.
          style={{
            flexShrink: 0,
            maxHeight: "calc(100dvh - 88px)",
            overflowY: "auto",
          }}
          pos="sticky"
          top={72}
          visibleFrom="sm"
        >
          <FilterPanel search={search} onChange={setSearch} />
        </Box>

        <Box style={{ flex: 1, minWidth: 0 }}>
          <Stack gap="md">
            <Button
              hiddenFrom="sm"
              variant="default"
              onClick={openFilters}
              data-testid="feed-filters-button"
              rightSection={
                activeFilterCount > 0 ? (
                  <Badge size="sm" variant="filled" circle>
                    {activeFilterCount}
                  </Badge>
                ) : undefined
              }
            >
              Filters
            </Button>
            <Group justify="space-between" align="center" wrap="wrap">
              <Group gap="sm" align="baseline" wrap="wrap">
                <Title order={2}>Series</Title>
                <Text size="sm" c="dimmed">
                  {list.isLoading
                    ? "loading…"
                    : `${total.toLocaleString()} match${total === 1 ? "" : "es"}`}
                </Text>
              </Group>
              <Group gap="sm" align="center" wrap="nowrap">
                <Select
                  size="xs"
                  w={110}
                  aria-label="Results per page"
                  data={PAGE_SIZE_OPTIONS.map((n) => ({
                    value: String(n),
                    label: `${n} / page`,
                  }))}
                  value={String(pageSize)}
                  onChange={(v) => {
                    setPageSize(Number(v) || DEFAULT_PAGE_SIZE);
                    // Reset to page 1: the current page may not exist at the
                    // new size. Page number stays in the URL (it's navigation).
                    setSearch({ ...search, page: 1 });
                  }}
                  allowDeselect={false}
                  data-testid="feed-page-size"
                />
                <Button
                  size="xs"
                  variant={wideMode ? "filled" : "default"}
                  onClick={toggleWideMode}
                  visibleFrom="lg"
                  aria-pressed={wideMode}
                  data-testid="feed-wide-toggle"
                >
                  Wide
                </Button>
                <SegmentedControl
                  size="xs"
                  value={view}
                  onChange={(v) => setView(v === "list" ? "list" : "card")}
                  data={[
                    { label: "Cards", value: "card" },
                    { label: "List", value: "list" },
                  ]}
                  data-testid="feed-view-toggle"
                />
              </Group>
            </Group>

            {list.isError && (
              <Alert color="red" title="Failed to load series">
                {(list.error as Error)?.message ?? "Unknown error"}
              </Alert>
            )}

            {list.isLoading && !list.data && (
              <Center py="xl">
                <Loader />
              </Center>
            )}

            {list.data && list.data.items.length === 0 && (
              <Alert color="gray" title="No series match these filters">
                Try clearing a filter or waiting for the next poll.
              </Alert>
            )}

            {isAdmin && selection.selected.size > 0 && (
              <SeriesSelectionBar
                selectedIds={[...selection.selected]}
                allPageSelected={selection.allPageSelected}
                somePageSelected={selection.somePageSelected}
                onToggleAll={selection.toggleAllOnPage}
                onClear={selection.clear}
              />
            )}

            {list.data && list.data.items.length > 0 && view === "card" && (
              // Fluid grid: a fixed min track width means freeing horizontal
              // space (wide mode, a bigger monitor) turns into more columns
              // rather than larger cards. ~175px keeps ~5 columns in the
              // centered layout and packs more when the container is wide.
              // `min(45%, 175px)` caps the min track at 45% of the container on
              // narrow phones, so the grid keeps at least 2 cards per row
              // instead of one oversized card, while desktop stays fluid.
              <Box
                data-testid="feed-card-grid"
                style={{
                  display: "grid",
                  gap: "var(--mantine-spacing-md)",
                  gridTemplateColumns:
                    "repeat(auto-fill, minmax(min(45%, 175px), 1fr))",
                }}
              >
                {list.data.items.map((s, i) => (
                  <SeriesCard
                    key={s.id}
                    series={s}
                    codexSynced={codexSynced}
                    selection={selectionFor(i, s.id)}
                  />
                ))}
              </Box>
            )}

            {list.data && list.data.items.length > 0 && view === "list" && (
              <Stack gap="xs" data-testid="feed-list-view">
                {list.data.items.map((s, i) => (
                  <SeriesListRow
                    key={s.id}
                    series={s}
                    codexSynced={codexSynced}
                    selection={selectionFor(i, s.id)}
                  />
                ))}
              </Stack>
            )}

            {totalPages > 1 && (
              <Center>
                <Pagination
                  value={search.page ?? 1}
                  onChange={(p) => setSearch({ ...search, page: p })}
                  total={totalPages}
                  size="sm"
                />
              </Center>
            )}
          </Stack>
        </Box>
      </Flex>

      <Drawer
        opened={filtersOpen}
        onClose={closeFilters}
        position="left"
        size="sm"
        // The FilterPanel renders its own "Filters" heading, so the drawer
        // chrome stays title-less (the close button still shows).
        aria-label="Filters"
        hiddenFrom="sm"
      >
        <FilterPanel search={search} onChange={setSearch} />
      </Drawer>
    </Container>
  );
}
