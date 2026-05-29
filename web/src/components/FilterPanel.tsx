import {
  ActionIcon,
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
  Text,
  TextInput,
  Title,
  Tooltip,
} from "@mantine/core";
import { useDebouncedCallback, useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { useEffect, useMemo, useState } from "react";
import { useGenres, useTags } from "@/api/queries";
import {
  type FilterSearch,
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
// so the user can type anything if they need to.
const KIND_OPTIONS = [
  "manga",
  "manhwa",
  "manhua",
  "novel",
  "one_shot",
  "other",
];
const STATUS_OPTIONS = [
  "ongoing",
  "completed",
  "hiatus",
  "cancelled",
  "unknown",
];
const SORT_OPTIONS = [
  { value: "last_release_at", label: "Last release" },
  { value: "first_seen_at", label: "First seen" },
  { value: "total_volumes", label: "Volume count" },
  { value: "total_chapters", label: "Chapter count" },
];

/// "Newest first / Oldest first" reads as nonsense for a numeric sort like
/// "Volume count", so the Order dropdown swaps its labels based on which
/// sort field is active. Date columns keep the recency wording; numeric
/// columns get "Highest first / Lowest first" instead.
type OrderLabels = { desc: string; asc: string };
const ORDER_LABELS_BY_SORT: Record<string, OrderLabels> = {
  last_release_at: { desc: "Newest first", asc: "Oldest first" },
  first_seen_at: { desc: "Newest first", asc: "Oldest first" },
  total_volumes: { desc: "Highest first", asc: "Lowest first" },
  total_chapters: { desc: "Highest first", asc: "Lowest first" },
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
  const presets = useFilterPresets((s) => s.presets);
  const savePreset = useFilterPresets((s) => s.savePreset);
  const deletePreset = useFilterPresets((s) => s.deletePreset);
  const [saveOpen, { open: openSave, close: closeSave }] = useDisclosure(false);
  const [presetName, setPresetName] = useState("");
  // Local mirror of the URL `q` so the input stays responsive while the
  // debounced commit catches up. Initialized from the URL and re-synced
  // when navigation changes `q` externally (e.g. preset apply, clear).
  const [qDraft, setQDraft] = useState(search.q ?? "");
  useEffect(() => {
    setQDraft(search.q ?? "");
  }, [search.q]);
  const genres = useGenres();
  const tags = useTags();

  const merge = (patch: Partial<FilterSearch>) =>
    onChange({ ...search, ...patch, page: 1 });

  const commitQ = useDebouncedCallback((next: string) => {
    const trimmed = next.trim();
    const current = search.q?.trim() ?? "";
    if (trimmed === current) return;
    onChange({ ...search, q: trimmed || undefined, page: 1 });
  }, SEARCH_DEBOUNCE_MS);

  const clearAll = () =>
    onChange({
      sort: search.sort,
      order: search.order,
      // Page size is a display preference, not a content filter — clearing
      // filters shouldn't reset how many results per page the user picked.
      pageSize: search.pageSize,
      page: 1,
    });

  const hasActiveFilters =
    Boolean(search.kind) ||
    Boolean(search.status) ||
    (search.genres?.length ?? 0) > 0 ||
    (search.tags?.length ?? 0) > 0 ||
    Boolean(search.q) ||
    typeof search.owned === "boolean" ||
    typeof search.hasReleases === "boolean";

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

  const handleSave = () => {
    const trimmed = presetName.trim();
    if (!trimmed) return;
    savePreset(trimmed, { ...search, page: 1 });
    notifications.show({
      message: `Preset "${trimmed}" saved`,
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
                {presets.map((p) => (
                  <Menu.Item
                    key={p.id}
                    onClick={() => onChange({ ...p.search, page: 1 })}
                    rightSection={
                      <Tooltip label="Delete preset">
                        <ActionIcon
                          variant="subtle"
                          size="xs"
                          c="red"
                          onClick={(e) => {
                            e.stopPropagation();
                            deletePreset(p.id);
                          }}
                          aria-label={`Delete preset ${p.name}`}
                        >
                          ×
                        </ActionIcon>
                      </Tooltip>
                    }
                  >
                    {p.name}
                  </Menu.Item>
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

        <Select
          label="Kind"
          placeholder="Any"
          data={KIND_OPTIONS}
          value={search.kind ?? null}
          onChange={(v) => merge({ kind: v ?? undefined })}
          clearable
          searchable
        />

        <Select
          label="Status"
          placeholder="Any"
          data={STATUS_OPTIONS}
          value={search.status ?? null}
          onChange={(v) => merge({ status: v ?? undefined })}
          clearable
          searchable
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

        {/*
          Ownership filter hidden until tsundoku actually talks to Codex.
          The `owned` flag exists in the schema + API, but with no Codex
          integration to flip it nothing ever sets `owned=true`, so the
          control would only ever filter to an empty set or no-op.
          Re-enable once the Codex HTTP sync lands.

          <Box>
            <Text size="sm" fw={500} mb={4}>
              Ownership
            </Text>
            <SegmentedControl
              size="xs"
              fullWidth
              value={triValue(search.owned)}
              onChange={(v) => merge({ owned: triParse(v) })}
              data={[
                { label: "Any", value: "any" },
                { label: "Discoverable", value: "false" },
                { label: "Owned", value: "true" },
              ]}
            />
          </Box>
        */}

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

        <Select
          label="Sort by"
          data={SORT_OPTIONS}
          value={search.sort ?? "last_release_at"}
          onChange={(v) => merge({ sort: v ?? undefined })}
          allowDeselect={false}
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
          <TextInput
            label="Name"
            placeholder="Ongoing manga, hidden owned…"
            value={presetName}
            onChange={(e) => setPresetName(e.currentTarget.value)}
            data-autofocus
            onKeyDown={(e) => {
              if (e.key === "Enter") handleSave();
            }}
          />
          <Group justify="flex-end">
            <Button variant="default" onClick={closeSave}>
              Cancel
            </Button>
            <Button onClick={handleSave} disabled={!presetName.trim()}>
              Save preset
            </Button>
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
