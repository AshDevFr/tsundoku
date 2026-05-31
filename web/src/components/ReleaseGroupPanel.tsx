import {
  Anchor,
  Box,
  Chip,
  Collapse,
  Group,
  Loader,
  Paper,
  SegmentedControl,
  Text,
  UnstyledButton,
} from "@mantine/core";
import { type ReleaseGroupFilters, useReleaseGroups } from "@/api/queries";

/// Breadth labels: how aggressively query variants collapse into one group.
/// Tight = primary cleaned query only; loose = any variant matches.
const BREADTH_OPTIONS = [
  { value: "1", label: "Tight" },
  { value: "2", label: "Medium" },
  { value: "3", label: "Loose" },
];

/// A quiet, collapsible helper above the review list: cluster the queue by
/// cleaned search query and let the operator click a cluster to filter the
/// list down to it (then bulk-link/reject the whole group). It's a shortcut
/// into the existing `searchQuery` filter, not a separate view — the active
/// chip mirrors the list's group scope.
export function ReleaseGroupPanel({
  open,
  onToggle,
  filters,
  breadth,
  onBreadth,
  activeQuery,
  onSelect,
  onClear,
}: {
  open: boolean;
  onToggle: () => void;
  /// The non-group filters in effect (title `q`, source, format, status); the
  /// clusters are computed within this scope but not within `activeQuery`.
  filters: ReleaseGroupFilters;
  breadth: number;
  onBreadth: (breadth: number) => void;
  /// The cluster the list is currently scoped to, or `null`.
  activeQuery: string | null;
  onSelect: (query: string) => void;
  onClear: () => void;
}) {
  // Only fetch when the panel is open. Breadth is part of the query key, so
  // switching it refetches the clusters at the new looseness.
  const groups = useReleaseGroups({ ...filters, breadth }, open);
  const items = groups.data?.groups ?? [];

  return (
    <Paper
      withBorder
      radius="md"
      p="xs"
      bg="var(--mantine-color-default)"
      data-testid="release-group-panel"
    >
      <Group justify="space-between" align="center" wrap="nowrap">
        <UnstyledButton
          onClick={onToggle}
          data-testid="release-group-toggle"
          aria-expanded={open}
        >
          <Group gap={6} align="center" wrap="nowrap">
            <Text size="sm" fw={500} c="dimmed">
              {open ? "▾" : "▸"} Group similar releases
            </Text>
            {!open && items.length > 0 && (
              <Text size="xs" c="dimmed">
                ({items.length})
              </Text>
            )}
          </Group>
        </UnstyledButton>
        {open && (
          <SegmentedControl
            size="xs"
            data={BREADTH_OPTIONS}
            value={String(breadth)}
            onChange={(v) => onBreadth(Number(v))}
            data-testid="release-group-breadth"
          />
        )}
      </Group>

      <Collapse expanded={open}>
        <Box pt="sm">
          {groups.isLoading && !groups.data ? (
            <Group gap="xs">
              <Loader size="xs" />
              <Text size="sm" c="dimmed">
                Clustering…
              </Text>
            </Group>
          ) : items.length === 0 ? (
            <Text size="sm" c="dimmed" data-testid="release-group-empty">
              No clusters at this breadth. Every queued release has a distinct
              cleaned query, or only one of each remains.
            </Text>
          ) : (
            <Group gap="xs">
              <Chip.Group
                multiple={false}
                value={activeQuery ?? ""}
                onChange={(v) => (v ? onSelect(v as string) : onClear())}
              >
                {items.map((g) => (
                  <Chip
                    key={g.query}
                    value={g.query}
                    size="xs"
                    variant="outline"
                    data-testid="release-group-chip"
                  >
                    {g.query}{" "}
                    <Text span c="dimmed">
                      ×{g.count}
                    </Text>
                  </Chip>
                ))}
              </Chip.Group>
              {activeQuery && (
                <Anchor
                  component="button"
                  type="button"
                  size="xs"
                  onClick={onClear}
                  data-testid="release-group-clear"
                >
                  Clear group
                </Anchor>
              )}
            </Group>
          )}
        </Box>
      </Collapse>
    </Paper>
  );
}
