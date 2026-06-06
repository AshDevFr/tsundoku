import {
  ActionIcon,
  Anchor,
  Badge,
  Button,
  Card,
  Checkbox,
  Collapse,
  Group,
  MultiSelect,
  SegmentedControl,
  Select,
  SimpleGrid,
  Stack,
  Switch,
  Text,
  Title,
  Tooltip,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { useState } from "react";
import { downloadSeriesExport, type ExportFormat } from "@/api/exportSeries";
import { useGenres, useTags } from "@/api/queries";
import {
  CODEX_STATUS_OPTIONS,
  KIND_OPTIONS,
  STATUS_OPTIONS,
} from "@/constants/series";

interface FieldDef {
  key: string;
  label: string;
  /// `canonicalTitle` is always exported (it's the one column that identifies
  /// a row), so its checkbox is checked and locked.
  alwaysOn?: boolean;
}

interface FieldGroup {
  group: string;
  fields: FieldDef[];
}

/// Field catalog, grouped to mirror the Codex export modal. The backend
/// (`ExportField`) is the authority on which keys are *valid*; this list owns
/// the human labels and grouping. Keep the keys in sync with the backend enum.
const FIELD_GROUPS: FieldGroup[] = [
  {
    group: "Identity",
    fields: [
      { key: "id", label: "Series ID" },
      { key: "canonicalTitle", label: "Title", alwaysOn: true },
      { key: "alternateTitles", label: "Alternate Titles" },
      { key: "coverUrl", label: "Cover URL" },
      { key: "metadataSource", label: "Metadata Source" },
      { key: "firstSeenAt", label: "First Seen" },
      { key: "lastReleaseAt", label: "Last Release" },
      { key: "metadataFetchedAt", label: "Metadata Updated" },
    ],
  },
  {
    group: "Metadata",
    fields: [
      { key: "kind", label: "Type" },
      { key: "status", label: "Status" },
      { key: "year", label: "Year" },
      { key: "description", label: "Summary" },
      { key: "genres", label: "Genres" },
      { key: "tags", label: "Tags" },
      { key: "externalIds", label: "External IDs" },
    ],
  },
  {
    group: "Availability",
    fields: [
      { key: "totalVolumes", label: "Published Volumes" },
      { key: "totalChapters", label: "Published Chapters" },
      { key: "highestVolume", label: "Available Volumes" },
      { key: "highestChapter", label: "Available Chapters" },
      { key: "releaseCount", label: "Release Count" },
    ],
  },
  {
    group: "Ratings",
    fields: [{ key: "rating", label: "Rating" }],
  },
  {
    group: "Ownership",
    fields: [
      { key: "owned", label: "Owned in Codex" },
      { key: "codexStatus", label: "Codex Status" },
    ],
  },
];

const ALL_KEYS = FIELD_GROUPS.flatMap((g) => g.fields.map((f) => f.key));

/// Fields off by default: ids, the cover URL, and the bookkeeping timestamps —
/// noise for a recommendation feed. Everything else is on. `canonicalTitle` is
/// always on regardless.
const DEFAULT_OFF = new Set([
  "id",
  "coverUrl",
  "metadataSource",
  "firstSeenAt",
  "lastReleaseAt",
  "metadataFetchedAt",
]);

function initialSelection(): Record<string, boolean> {
  const sel: Record<string, boolean> = {};
  for (const key of ALL_KEYS) sel[key] = !DEFAULT_OFF.has(key);
  sel.canonicalTitle = true;
  return sel;
}

const FORMAT_DATA: { label: string; value: ExportFormat }[] = [
  { label: "JSON", value: "json" },
  { label: "CSV", value: "csv" },
  { label: "Markdown", value: "markdown" },
];

const METADATA_SOURCE_DATA = [
  { value: "manual", label: "Manual only" },
  { value: "auto", label: "Provider-backed only" },
];

const HAS_RELEASES_DATA = [
  { value: "true", label: "Has releases" },
  { value: "false", label: "No releases (orphaned)" },
];

/// Admin catalog-export page. Mirrors the Codex export modal: pick a format,
/// scope with the same filters as the browse list, choose fields, and download
/// the whole filtered catalog as one file to feed an LLM agent.
export function AdminExportPage() {
  const genres = useGenres();
  const tags = useTags();

  const [format, setFormat] = useState<ExportFormat>("json");
  const [includeReleases, setIncludeReleases] = useState(false);
  const [selected, setSelected] =
    useState<Record<string, boolean>>(initialSelection);
  const [running, setRunning] = useState(false);

  // Filters. Collapsed by default — the whole-catalog dump is the common case,
  // so filters are an opt-in drawer rather than always-on noise.
  const [filtersOpen, { toggle: toggleFilters }] = useDisclosure(false);
  const [kinds, setKinds] = useState<string[]>([]);
  const [statuses, setStatuses] = useState<string[]>([]);
  const [metadataSource, setMetadataSource] = useState<string | null>(null);
  const [hasReleases, setHasReleases] = useState<string | null>(null);
  const [codexStatus, setCodexStatus] = useState<string[]>([]);
  const [selectedGenres, setSelectedGenres] = useState<string[]>([]);
  const [selectedTags, setSelectedTags] = useState<string[]>([]);

  // How many filter dimensions are constraining the export — shown in the
  // collapsed header so an active filter isn't hidden out of sight.
  const activeFilterCount =
    (kinds.length > 0 ? 1 : 0) +
    (statuses.length > 0 ? 1 : 0) +
    (metadataSource ? 1 : 0) +
    (hasReleases ? 1 : 0) +
    (codexStatus.length > 0 ? 1 : 0) +
    (selectedGenres.length > 0 ? 1 : 0) +
    (selectedTags.length > 0 ? 1 : 0);

  // Releases nest only in JSON/Markdown; CSV is a flat series-level table.
  const releasesDisabled = format === "csv";
  const effectiveIncludeReleases = !releasesDisabled && includeReleases;

  const genreNames = genres.data?.items.map((g) => g.name) ?? [];
  const tagNames = tags.data?.items.map((t) => t.name) ?? [];

  const selectedCount = ALL_KEYS.filter((k) => selected[k]).length;

  const toggleField = (key: string) => {
    if (key === "canonicalTitle") return; // locked
    setSelected((prev) => ({ ...prev, [key]: !prev[key] }));
  };

  const setAll = (value: boolean) => {
    const next: Record<string, boolean> = {};
    for (const key of ALL_KEYS) next[key] = value;
    next.canonicalTitle = true; // always included
    setSelected(next);
  };

  const handleExport = async () => {
    const fields = ALL_KEYS.filter(
      (k) => selected[k] || k === "canonicalTitle",
    );
    setRunning(true);
    try {
      await downloadSeriesExport({
        format,
        fields,
        includeReleases: effectiveIncludeReleases,
        filters: {
          kind: kinds,
          status: statuses,
          metadataSource,
          hasReleases: hasReleases == null ? null : hasReleases === "true",
          codexStatus,
          genres: selectedGenres,
          tags: selectedTags,
        },
      });
      notifications.show({
        color: "blue",
        title: "Export started",
        message: `Downloading ${format.toUpperCase()} (${fields.length} field(s)).`,
      });
    } catch (e) {
      notifications.show({
        color: "red",
        title: "Export failed",
        message: (e as Error).message,
      });
    } finally {
      setRunning(false);
    }
  };

  return (
    <Stack gap="lg">
      <Stack gap={4}>
        <Title order={3}>Export catalog</Title>
        <Text size="sm" c="dimmed">
          Download the discovery catalog as a single file to feed an LLM agent
          ("here is what exists, what's available, and what I don't already
          own"). Filters scope which series are included; an unfiltered export
          dumps the whole catalog.
        </Text>
      </Stack>

      <Card withBorder radius="md" p="md" data-testid="export-format-card">
        <Stack gap="sm">
          <Title order={4}>Format</Title>
          <SegmentedControl
            data={FORMAT_DATA}
            value={format}
            onChange={(v) => setFormat(v as ExportFormat)}
            data-testid="export-format"
          />
          <Tooltip
            label="CSV is a flat series-level table; releases nest only in JSON and Markdown."
            disabled={!releasesDisabled}
          >
            <Switch
              label="Include linked releases (JSON / Markdown only)"
              checked={effectiveIncludeReleases}
              onChange={(e) => setIncludeReleases(e.currentTarget.checked)}
              disabled={releasesDisabled}
              data-testid="export-include-releases"
            />
          </Tooltip>
        </Stack>
      </Card>

      <Card withBorder radius="md" p="md" data-testid="export-filters-card">
        <Stack gap="sm">
          <Group
            justify="space-between"
            align="center"
            onClick={toggleFilters}
            style={{ cursor: "pointer" }}
            data-testid="export-filters-toggle"
          >
            <Group gap="xs" align="center">
              <Title order={4}>Filters</Title>
              {activeFilterCount > 0 && (
                <Badge
                  size="sm"
                  variant="light"
                  data-testid="export-filters-count"
                >
                  {activeFilterCount} active
                </Badge>
              )}
              {activeFilterCount === 0 && (
                <Text size="xs" c="dimmed">
                  whole catalog
                </Text>
              )}
            </Group>
            <ActionIcon
              variant="subtle"
              color="gray"
              aria-label={filtersOpen ? "Collapse filters" : "Expand filters"}
            >
              <Text size="sm" aria-hidden>
                {filtersOpen ? "▲" : "▼"}
              </Text>
            </ActionIcon>
          </Group>
          <Collapse expanded={filtersOpen}>
            <Stack gap="sm">
              <Text size="xs" c="dimmed">
                Same semantics as the browse list. Leave blank for the whole
                catalog. Tip: set Codex status to{" "}
                <Text span fw={600}>
                  Not on Codex
                </Text>{" "}
                to export only series you don't own.
              </Text>
              <SimpleGrid cols={{ base: 1, sm: 2 }} spacing="sm">
                <MultiSelectField
                  label="Type"
                  data={KIND_OPTIONS}
                  value={kinds}
                  onChange={setKinds}
                  testid="export-filter-kind"
                />
                <MultiSelectField
                  label="Status"
                  data={STATUS_OPTIONS}
                  value={statuses}
                  onChange={setStatuses}
                  testid="export-filter-status"
                />
                <Select
                  label="Metadata source"
                  data={METADATA_SOURCE_DATA}
                  value={metadataSource}
                  onChange={setMetadataSource}
                  clearable
                  placeholder="Any"
                  data-testid="export-filter-metadata-source"
                />
                <Select
                  label="Has releases"
                  data={HAS_RELEASES_DATA}
                  value={hasReleases}
                  onChange={setHasReleases}
                  clearable
                  placeholder="Any"
                  data-testid="export-filter-has-releases"
                />
                <MultiSelectField
                  label="Codex status"
                  data={CODEX_STATUS_OPTIONS}
                  value={codexStatus}
                  onChange={setCodexStatus}
                  testid="export-filter-codex-status"
                />
                <MultiSelectField
                  label="Genres"
                  data={genreNames}
                  value={selectedGenres}
                  onChange={setSelectedGenres}
                  testid="export-filter-genres"
                />
                <MultiSelectField
                  label="Tags"
                  data={tagNames}
                  value={selectedTags}
                  onChange={setSelectedTags}
                  testid="export-filter-tags"
                />
              </SimpleGrid>
            </Stack>
          </Collapse>
        </Stack>
      </Card>

      <Card withBorder radius="md" p="md" data-testid="export-fields-card">
        <Stack gap="sm">
          <Group justify="space-between" align="center">
            <Stack gap={2}>
              <Title order={4}>Fields</Title>
              <Text size="xs" c="dimmed">
                Series title is always included. {selectedCount} of{" "}
                {ALL_KEYS.length} optional field(s) selected.
              </Text>
            </Stack>
            <Group gap="xs">
              <Anchor
                component="button"
                type="button"
                size="sm"
                onClick={() => setAll(true)}
                data-testid="export-select-all"
              >
                Select all
              </Anchor>
              <Anchor
                component="button"
                type="button"
                size="sm"
                c="dimmed"
                onClick={() => setAll(false)}
                data-testid="export-clear"
              >
                Clear
              </Anchor>
            </Group>
          </Group>

          {FIELD_GROUPS.map((group) => (
            <Card
              key={group.group}
              withBorder
              radius="sm"
              p="sm"
              bg="var(--mantine-color-default)"
            >
              <Stack gap="xs">
                <Text size="sm" fw={600} c="dimmed">
                  {group.group}
                </Text>
                <Group gap="md">
                  {group.fields.map((field) => (
                    <Checkbox
                      key={field.key}
                      label={field.label}
                      checked={field.alwaysOn ? true : !!selected[field.key]}
                      disabled={field.alwaysOn}
                      onChange={() => toggleField(field.key)}
                      data-testid={`export-field-${field.key}`}
                    />
                  ))}
                </Group>
              </Stack>
            </Card>
          ))}
        </Stack>
      </Card>

      <Group justify="flex-end">
        <Button
          onClick={handleExport}
          loading={running}
          data-testid="export-button"
        >
          Export
        </Button>
      </Group>
    </Stack>
  );
}

/// Thin wrapper so the `data-testid` is forwarded onto the MultiSelect (Mantine
/// spreads unknown props onto the root) and the three multi-selects share one
/// shape. `data` accepts either bare strings or `{value,label}` option objects.
function MultiSelectField({
  label,
  data,
  value,
  onChange,
  testid,
}: {
  label: string;
  data: (string | { value: string; label: string })[];
  value: string[];
  onChange: (v: string[]) => void;
  testid: string;
}) {
  return (
    <MultiSelect
      label={label}
      data={data}
      value={value}
      onChange={onChange}
      clearable
      searchable
      placeholder={value.length === 0 ? "Any" : undefined}
      data-testid={testid}
    />
  );
}
