import {
  ActionIcon,
  Box,
  Button,
  Group,
  Menu,
  Modal,
  Paper,
  Select,
  Stack,
  Switch,
  Text,
  TextInput,
  Title,
  Tooltip,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { useState } from "react";
import { type FilterSearch, useFilterPresets } from "@/stores/filters";

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
];

export function FilterPanel({ search, onChange }: FilterPanelProps) {
  const presets = useFilterPresets((s) => s.presets);
  const savePreset = useFilterPresets((s) => s.savePreset);
  const deletePreset = useFilterPresets((s) => s.deletePreset);
  const [saveOpen, { open: openSave, close: closeSave }] = useDisclosure(false);
  const [presetName, setPresetName] = useState("");

  const merge = (patch: Partial<FilterSearch>) =>
    onChange({ ...search, ...patch, page: 1 });

  const clearAll = () =>
    onChange({ sort: search.sort, order: search.order, page: 1 });

  const hasActiveFilters =
    Boolean(search.kind) ||
    Boolean(search.status) ||
    typeof search.owned === "boolean";

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

        <Box>
          <Text size="sm" fw={500} mb={4}>
            Ownership
          </Text>
          <Group gap="xs">
            <Switch
              size="sm"
              label="Hide owned"
              checked={search.owned === false}
              onChange={(e) =>
                merge({ owned: e.currentTarget.checked ? false : undefined })
              }
            />
            <Switch
              size="sm"
              label="Owned only"
              checked={search.owned === true}
              onChange={(e) =>
                merge({ owned: e.currentTarget.checked ? true : undefined })
              }
            />
          </Group>
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
            { value: "desc", label: "Newest first" },
            { value: "asc", label: "Oldest first" },
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
