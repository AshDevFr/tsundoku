import {
  ActionIcon,
  Autocomplete,
  Box,
  Button,
  CloseButton,
  Group,
  Menu,
  Modal,
  MultiSelect,
  Paper,
  SegmentedControl,
  Select,
  Stack,
  Switch,
  Text,
  TextInput,
  Title,
  Tooltip,
} from "@mantine/core";
import { useDebouncedCallback, useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { useEffect, useMemo, useState } from "react";
import { useGenres, useSourceCounts, useTags } from "@/api/queries";
import {
  CODEX_STATUS_OPTIONS,
  KIND_OPTIONS,
  STATUS_OPTIONS,
} from "@/constants/series";
import { useAdminAuth } from "@/stores/auth";
import {
  countActiveFilters,
  type FilterSearch,
  sortPresets,
  type TagFilterMode,
  useFilterPresets,
} from "@/stores/filters";

const SEARCH_DEBOUNCE_MS = 250;

interface FilterPanelProps {
  search: FilterSearch;
  onChange: (next: FilterSearch) => void;
}

// Vocabularies are open-ended (the backend doesn't restrict them), but in
// practice these are the values the resolver writes. Use combobox-style selects
// so the user can type anything if they need to. The option lists live in
// `@/constants/series` so the manual-series editor shares the exact vocab.
const SORT_OPTIONS = [
  { value: "last_release_at", label: "Last release" },
  // When tsundoku found the release, not when it was posted upstream. A query
  // feed, a backfill, or the per-series release search routinely surfaces
  // posts that are months or years old, and those sort to the bottom under
  // "Last release" however recently they were discovered.
  { value: "last_discovered_at", label: "Recently discovered" },
  { value: "published_start_date", label: "Publication date" },
  { value: "first_seen_at", label: "First seen" },
  { value: "total_volumes", label: "Volume count" },
  { value: "total_chapters", label: "Chapter count" },
  { value: "highest_volume", label: "Available volumes" },
  { value: "highest_chapter", label: "Available chapters" },
  { value: "rating", label: "Rating" },
];

/// "Newest first / Oldest first" reads as nonsense for a numeric sort like
/// "Volume count", so the Order dropdown swaps its labels based on which
/// sort field is active. Date columns keep the recency wording; numeric
/// columns get "Highest first / Lowest first" instead.
type OrderLabels = { desc: string; asc: string };
const ORDER_LABELS_BY_SORT: Record<string, OrderLabels> = {
  last_release_at: { desc: "Newest first", asc: "Oldest first" },
  last_discovered_at: { desc: "Newest first", asc: "Oldest first" },
  published_start_date: { desc: "Newest first", asc: "Oldest first" },
  first_seen_at: { desc: "Newest first", asc: "Oldest first" },
  total_volumes: { desc: "Highest first", asc: "Lowest first" },
  total_chapters: { desc: "Highest first", asc: "Lowest first" },
  highest_volume: { desc: "Highest first", asc: "Lowest first" },
  highest_chapter: { desc: "Highest first", asc: "Lowest first" },
  rating: { desc: "Highest first", asc: "Lowest first" },
};
const DEFAULT_ORDER_LABELS: OrderLabels = {
  desc: "Highest first",
  asc: "Lowest first",
};

// SegmentedControl needs a string value, but boolean tri-state filters
// use `true | false | undefined` (with `undefined` meaning "no
// constraint"). These two helpers bridge the two representations so the
// rest of the panel doesn't have to think about it.
function triValue(b: boolean | undefined): "any" | "true" | "false" {
  if (b === true) return "true";
  if (b === false) return "false";
  return "any";
}
function triParse(v: string): boolean | undefined {
  if (v === "true") return true;
  if (v === "false") return false;
  return undefined;
}

export function FilterPanel({ search, onChange }: FilterPanelProps) {
  // The Codex filter is admin-only; the backend ignores it without a valid
  // token, so there's no point showing it to anon sessions.
  const isAdmin = useAdminAuth((s) => Boolean(s.token));
  const presets = useFilterPresets((s) => s.presets);
  const activePresetId = useFilterPresets((s) => s.activePresetId);
  const setActivePreset = useFilterPresets((s) => s.setActivePreset);
  const savePreset = useFilterPresets((s) => s.savePreset);
  const updatePreset = useFilterPresets((s) => s.updatePreset);
  const deletePreset = useFilterPresets((s) => s.deletePreset);
  const activePreset = useMemo(
    () => presets.find((p) => p.id === activePresetId),
    [presets, activePresetId],
  );
  const [saveOpen, { open: openSaveModal, close: closeSaveModal }] =
    useDisclosure(false);
  const [presetName, setPresetName] = useState("");
  // Two-step guard so overwriting an existing preset needs an explicit
  // second click rather than silently clobbering it. Update gets its own flag
  // for the same reason: arming one button must not arm the other.
  const [confirmOverwrite, setConfirmOverwrite] = useState(false);
  const [confirmUpdate, setConfirmUpdate] = useState(false);
  const closeSave = () => {
    setConfirmOverwrite(false);
    setConfirmUpdate(false);
    closeSaveModal();
  };
  const openSave = () => {
    // Seed from the preset the operator loaded, so updating it is a click
    // rather than an exercise in reproducing its name exactly.
    setPresetName(activePreset?.name ?? "");
    setConfirmOverwrite(false);
    setConfirmUpdate(false);
    openSaveModal();
  };
  // Local mirror of the URL `q` so the input stays responsive while the
  // debounced commit catches up. Initialized from the URL and re-synced
  // when navigation changes `q` externally (e.g. preset apply, clear).
  const [qDraft, setQDraft] = useState(search.q ?? "");
  useEffect(() => {
    setQDraft(search.q ?? "");
  }, [search.q]);
  const genres = useGenres();
  const tags = useTags();
  // Admin-only; the hook self-disables for anon sessions, so this stays empty
  // and the control below is never rendered.
  const sourceCounts = useSourceCounts();

  const merge = (patch: Partial<FilterSearch>) =>
    onChange({ ...search, ...patch, page: 1 });

  const commitQ = useDebouncedCallback((next: string) => {
    // Don't trim here: trimming the committed value re-syncs `qDraft` back
    // to a space-stripped string (via the effect above), so a trailing space
    // typed between words is wiped and the next word collides. The backend
    // trims `q` server-side, so the raw value round-trips harmlessly.
    const current = search.q ?? "";
    if (next === current) return;
    onChange({ ...search, q: next || undefined, page: 1 });
  }, SEARCH_DEBOUNCE_MS);

  const clearAll = () => {
    // Clearing filters abandons the loaded preset. Tweaking an individual
    // filter deliberately does not, so the save modal can still offer to
    // write the tweak back into it.
    setActivePreset(undefined);
    onChange({
      sort: search.sort,
      order: search.order,
      page: 1,
    });
  };

  const hasActiveFilters = countActiveFilters(search) > 0;

  const activeSort = search.sort ?? "last_release_at";
  const orderLabels = ORDER_LABELS_BY_SORT[activeSort] ?? DEFAULT_ORDER_LABELS;

  // Sort vocab alphabetically (case-insensitive) — the backend returns
  // these sorted by usage which is great for an autocomplete but reads
  // as scrambled when you display every option as a chip.
  const genreItems = useMemo(
    () =>
      (genres.data?.items ?? [])
        .slice()
        .sort((a, b) =>
          a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
        ),
    [genres.data?.items],
  );
  const tagItems = useMemo(
    () =>
      (tags.data?.items ?? [])
        .slice()
        .sort((a, b) =>
          a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
        ),
    [tags.data?.items],
  );
  // Feed names sorted alphabetically (the endpoint returns them usage-sorted,
  // which reads as scrambled in a chip list), with the series count in the
  // label so the operator can see each feed's reach at a glance.
  const sourceData = useMemo(
    () =>
      (sourceCounts.data?.items ?? [])
        .slice()
        .sort((a, b) =>
          a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
        )
        .map((s) => ({ value: s.name, label: `${s.name} (${s.seriesCount})` })),
    [sourceCounts.data?.items],
  );

  const sortedPresets = useMemo(() => sortPresets(presets), [presets]);

  const trimmedName = presetName.trim();
  const overwriting = useMemo(
    () =>
      presets.find((p) => p.name.toLowerCase() === trimmedName.toLowerCase()),
    [presets, trimmedName],
  );

  // Update targets the loaded preset, so it is only offered while the field
  // still names it. Editing the name means the operator is describing a new
  // preset, and an "Update <old name>" button next to a different name reads
  // as a mistake. Save as new stands down in the reverse case, so exactly one
  // write button is meaningful at a time.
  const savingOverLoaded = Boolean(
    activePreset && trimmedName === activePreset.name,
  );

  const nameDescription = useMemo(() => {
    if (activePreset && savingOverLoaded)
      return `Update writes the current filters into "${activePreset.name}".`;
    if (overwriting)
      return `This name is taken — saving will overwrite "${overwriting.name}".`;
    return undefined;
  }, [activePreset, savingOverLoaded, overwriting]);

  const saveLabel = useMemo(() => {
    if (savingOverLoaded) return "Save as new";
    if (overwriting)
      return confirmOverwrite
        ? "Click again to confirm"
        : `Overwrite "${overwriting.name}"`;
    return activePreset ? "Save as new" : "Save preset";
  }, [savingOverLoaded, overwriting, confirmOverwrite, activePreset]);

  const handleSave = () => {
    if (!trimmedName) return;
    if (overwriting && !confirmOverwrite) {
      setConfirmOverwrite(true);
      return;
    }
    savePreset(trimmedName, { ...search, page: 1 });
    notifications.show({
      message: overwriting
        ? `Preset "${trimmedName}" updated`
        : `Preset "${trimmedName}" saved`,
      color: "green",
    });
    setPresetName("");
    closeSave();
  };

  const handleUpdate = () => {
    if (!activePreset || !savingOverLoaded) return;
    if (!confirmUpdate) {
      setConfirmUpdate(true);
      return;
    }
    updatePreset(activePreset.id, { ...search, page: 1 });
    notifications.show({
      message: `Preset "${activePreset.name}" updated`,
      color: "green",
    });
    setPresetName("");
    closeSave();
  };

  return (
    <Paper withBorder radius="md" p="md">
      <Stack gap="md">
        <Group justify="space-between" align="center">
          <Title order={5}>Filters</Title>
          <Group gap={4}>
            <Menu position="bottom-end" withinPortal>
              <Menu.Target>
                <Button
                  variant="subtle"
                  size="xs"
                  disabled={presets.length === 0}
                >
                  Presets
                </Button>
              </Menu.Target>
              <Menu.Dropdown>
                {presets.length === 0 && (
                  <Menu.Label>No saved presets</Menu.Label>
                )}
                {sortedPresets.map((p) => (
                  <Group key={p.id} gap={0} wrap="nowrap" pr="xs">
                    <Menu.Item
                      flex={1}
                      aria-current={
                        p.id === activePresetId ? "true" : undefined
                      }
                      leftSection={
                        <Text size="xs" c="dimmed" w={8} aria-hidden>
                          {p.id === activePresetId ? "✓" : ""}
                        </Text>
                      }
                      onClick={() => {
                        setActivePreset(p.id);
                        onChange({ ...p.search, page: 1 });
                      }}
                    >
                      {p.name}
                    </Menu.Item>
                    <Tooltip label="Delete preset">
                      <ActionIcon
                        variant="subtle"
                        size="xs"
                        c="red"
                        onClick={() => deletePreset(p.id)}
                        aria-label={`Delete preset ${p.name}`}
                      >
                        ×
                      </ActionIcon>
                    </Tooltip>
                  </Group>
                ))}
              </Menu.Dropdown>
            </Menu>
            <Button variant="subtle" size="xs" onClick={openSave}>
              Save
            </Button>
          </Group>
        </Group>

        <TextInput
          label="Search"
          placeholder="Title or author…"
          value={qDraft}
          onChange={(e) => {
            const next = e.currentTarget.value;
            setQDraft(next);
            commitQ(next);
          }}
          rightSection={
            qDraft ? (
              <CloseButton
                size="sm"
                aria-label="Clear search"
                onClick={() => {
                  setQDraft("");
                  commitQ("");
                }}
              />
            ) : null
          }
          data-testid="feed-search-input"
        />

        <Switch
          label="Also search descriptions"
          checked={search.searchDescriptions ?? false}
          onChange={(e) =>
            merge({
              searchDescriptions: e.currentTarget.checked || undefined,
            })
          }
          data-testid="feed-search-descriptions-toggle"
        />

        <MultiSelect
          label="Kind"
          placeholder={search.kind?.length ? undefined : "Any"}
          description="Matches any selected kind"
          data={KIND_OPTIONS}
          value={search.kind ?? []}
          onChange={(v) => merge({ kind: v.length > 0 ? v : undefined })}
          clearable
          searchable
          data-testid="filter-kind"
        />

        <MultiSelect
          label="Status"
          placeholder={search.status?.length ? undefined : "Any"}
          description="Matches any selected status"
          data={STATUS_OPTIONS}
          value={search.status ?? []}
          onChange={(v) => merge({ status: v.length > 0 ? v : undefined })}
          clearable
          searchable
          data-testid="filter-status"
        />

        <ComboFilterGroup
          label="Genres"
          testId="filter-genres"
          items={genreItems}
          loading={genres.isLoading}
          selected={search.genres ?? []}
          mode={search.genresMode ?? "any"}
          onSelectedChange={(genres) =>
            merge({ genres: genres.length > 0 ? genres : undefined })
          }
          onModeChange={(m) =>
            merge({ genresMode: m === "any" ? undefined : m })
          }
        />

        <ComboFilterGroup
          label="Tags"
          testId="filter-tags"
          items={tagItems}
          loading={tags.isLoading}
          selected={search.tags ?? []}
          mode={search.tagsMode ?? "any"}
          onSelectedChange={(tags) =>
            merge({ tags: tags.length > 0 ? tags : undefined })
          }
          onModeChange={(m) => merge({ tagsMode: m === "any" ? undefined : m })}
        />

        {isAdmin && (
          <MultiSelect
            label="Codex"
            placeholder={search.codexStatus?.length ? undefined : "Any"}
            description="Matches any selected status"
            data={CODEX_STATUS_OPTIONS}
            value={search.codexStatus ?? []}
            onChange={(v) =>
              merge({ codexStatus: v.length > 0 ? v : undefined })
            }
            clearable
            data-testid="filter-codex-status"
          />
        )}

        {isAdmin && (
          <MultiSelect
            label="Sources"
            placeholder={search.sources?.length ? undefined : "Any"}
            description="Series with a release from any selected source"
            data={sourceData}
            value={search.sources ?? []}
            onChange={(v) => merge({ sources: v.length > 0 ? v : undefined })}
            clearable
            searchable
            data-testid="filter-sources"
          />
        )}

        {isAdmin && (
          <Box>
            <Text size="sm" fw={500} mb={4}>
              Wishlist
            </Text>
            <SegmentedControl
              size="xs"
              fullWidth
              value={triValue(search.wishlisted)}
              onChange={(v) => merge({ wishlisted: triParse(v) })}
              data={[
                { label: "Any", value: "any" },
                { label: "Not wishlisted", value: "false" },
                { label: "Wishlisted", value: "true" },
              ]}
              data-testid="filter-wishlisted"
            />
          </Box>
        )}

        <Box>
          <Text size="sm" fw={500} mb={4}>
            Releases
          </Text>
          <SegmentedControl
            size="xs"
            fullWidth
            value={triValue(search.hasReleases)}
            onChange={(v) => merge({ hasReleases: triParse(v) })}
            data={[
              { label: "Any", value: "any" },
              { label: "Orphans", value: "false" },
              { label: "Has releases", value: "true" },
            ]}
          />
        </Box>

        <Box>
          <Text size="sm" fw={500} mb={4}>
            Source
          </Text>
          <SegmentedControl
            size="xs"
            fullWidth
            value={search.metadataSource ?? "any"}
            onChange={(v) =>
              merge({
                metadataSource: v === "manual" || v === "auto" ? v : undefined,
              })
            }
            data={[
              { label: "Any", value: "any" },
              { label: "Auto", value: "auto" },
              { label: "Manual", value: "manual" },
            ]}
            data-testid="filter-metadata-source"
          />
        </Box>

        <Select
          label="Sort by"
          data={SORT_OPTIONS}
          value={search.sort ?? "last_release_at"}
          onChange={(v) => merge({ sort: v ?? undefined })}
          allowDeselect={false}
          data-testid="filter-sort"
        />
        <Select
          label="Order"
          data={[
            { value: "desc", label: orderLabels.desc },
            { value: "asc", label: orderLabels.asc },
          ]}
          value={search.order ?? "desc"}
          onChange={(v) => merge({ order: v ?? undefined })}
          allowDeselect={false}
        />

        <Button
          variant="subtle"
          size="xs"
          onClick={clearAll}
          disabled={!hasActiveFilters}
        >
          Clear filters
        </Button>
      </Stack>

      <Modal
        opened={saveOpen}
        onClose={closeSave}
        title="Save filter preset"
        centered
      >
        <Stack>
          <Autocomplete
            label="Name"
            placeholder="Ongoing manga, hidden owned…"
            data={sortedPresets.map((p) => p.name)}
            clearable
            // The field is autofocused, and Mantine opens the dropdown on
            // focus by default. With the name prefilled that means the modal
            // opens under a suggestion list containing the value already in
            // the field. Open it on typing instead.
            openOnFocus={false}
            // Mantine hides the clear button from the accessibility tree by
            // default. It stays out of the tab order (tabIndex -1), but a
            // labelled button is discoverable to a screen reader's virtual
            // cursor, which is worth more here than hiding it.
            clearButtonProps={{
              "aria-label": "Clear preset name",
              "aria-hidden": false,
            }}
            value={presetName}
            onChange={(value) => {
              setPresetName(value);
              setConfirmOverwrite(false);
              setConfirmUpdate(false);
            }}
            data-autofocus
            onKeyDown={(e) => {
              if (e.key === "Enter") handleSave();
            }}
            description={nameDescription}
          />
          <Group justify="flex-end">
            <Button variant="default" onClick={closeSave}>
              Cancel
            </Button>
            <Button
              onClick={handleSave}
              disabled={!trimmedName || savingOverLoaded}
              color={overwriting && !savingOverLoaded ? "yellow" : undefined}
            >
              {saveLabel}
            </Button>
            {activePreset && savingOverLoaded && (
              <Button color="yellow" onClick={handleUpdate}>
                {confirmUpdate
                  ? "Click again to confirm"
                  : `Update "${activePreset.name}"`}
              </Button>
            )}
          </Group>
        </Stack>
      </Modal>
    </Paper>
  );
}

interface ComboFilterItem {
  name: string;
  seriesCount: number;
}

interface ComboFilterGroupProps {
  label: string;
  testId: string;
  items: ComboFilterItem[];
  loading: boolean;
  selected: string[];
  mode: TagFilterMode;
  onSelectedChange: (next: string[]) => void;
  onModeChange: (next: TagFilterMode) => void;
}

/// Searchable multi-select for genre/tag vocabularies. Tags ship 5k+
/// values; a chip cloud would mount every option into the DOM and
/// re-reconcile the whole set on each parent re-render (e.g. every search
/// keystroke), which locks the main thread. The combobox only renders the
/// matching slice (`limit`) inside an open dropdown, so typing stays cheap
/// no matter how large the vocabulary grows.
function ComboFilterGroup({
  label,
  testId,
  items,
  loading,
  selected,
  mode,
  onSelectedChange,
  onModeChange,
}: ComboFilterGroupProps) {
  const data = useMemo(
    () => items.map((i) => ({ value: i.name, label: i.name })),
    [items],
  );
  const counts = useMemo(() => {
    const m = new Map<string, number>();
    for (const i of items) m.set(i.name, i.seriesCount);
    return m;
  }, [items]);

  return (
    <Box data-testid={testId}>
      <Group justify="space-between" align="center" mb={6} wrap="nowrap">
        <Group gap={6} align="center" wrap="nowrap">
          <Text size="sm" fw={500}>
            {label}
          </Text>
          {selected.length > 0 && (
            <Tooltip label={`Clear ${label.toLowerCase()}`} withinPortal>
              <CloseButton
                size="xs"
                aria-label={`Clear ${label.toLowerCase()}`}
                onClick={() => onSelectedChange([])}
              />
            </Tooltip>
          )}
        </Group>
        <SegmentedControl
          size="xs"
          value={mode}
          onChange={(v) => onModeChange(v === "all" ? "all" : "any")}
          data={[
            { label: "All", value: "all" },
            { label: "Any", value: "any" },
          ]}
          disabled={selected.length < 2}
        />
      </Group>
      <MultiSelect
        data={data}
        value={selected}
        onChange={onSelectedChange}
        searchable
        clearable
        hidePickedOptions
        limit={100}
        maxDropdownHeight={260}
        placeholder={loading ? "loading…" : `Search ${label.toLowerCase()}…`}
        nothingFoundMessage="No matches"
        renderOption={({ option }) => (
          <Group justify="space-between" gap="xs" w="100%" wrap="nowrap">
            <Text size="sm">{option.label}</Text>
            <Text size="xs" c="dimmed">
              {counts.get(option.value) ?? 0}
            </Text>
          </Group>
        )}
      />
    </Box>
  );
}
