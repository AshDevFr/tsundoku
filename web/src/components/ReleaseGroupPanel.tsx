import {
  Box,
  Collapse,
  Group,
  Loader,
  Paper,
  ScrollArea,
  SegmentedControl,
  Stack,
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
            // One group per line, each row contained within the panel width so
            // a long candidate title is readable but never overflows. Clicking
            // an active row clears it. Capped at ~10 rows tall; the rest scroll
            // so a large queue can't push the review list off-screen.
            <ScrollArea.Autosize mah={320} type="auto" offsetScrollbars>
              <Stack gap={4}>
                {items.map((g) => {
                  const hint = g.topCandidate?.title;
                  const active = g.query === activeQuery;
                  const full = hint ? `${g.query} → ${hint}` : g.query;
                  return (
                    <UnstyledButton
                      key={g.query}
                      title={full}
                      onClick={() => (active ? onClear() : onSelect(g.query))}
                      data-testid="release-group-chip"
                      data-active={active || undefined}
                      style={{
                        display: "block",
                        width: "100%",
                        borderRadius: "var(--mantine-radius-sm)",
                        border: "1px solid var(--mantine-color-default-border)",
                        padding: "3px 8px",
                        backgroundColor: active
                          ? "var(--mantine-color-cyan-light)"
                          : undefined,
                      }}
                    >
                      <Group gap={6} wrap="nowrap" w="100%">
                        <Text size="sm" style={{ flex: "none" }}>
                          {g.query}
                        </Text>
                        <Text size="sm" c="dimmed" style={{ flex: "none" }}>
                          ×{g.count}
                        </Text>
                        {hint && (
                          <Text
                            size="sm"
                            c="dimmed"
                            fs="italic"
                            truncate
                            style={{ minWidth: 0 }}
                          >
                            → {hint}
                          </Text>
                        )}
                        {active && (
                          <Text
                            size="sm"
                            c="dimmed"
                            ml="auto"
                            style={{ flex: "none" }}
                            aria-hidden
                          >
                            ✕
                          </Text>
                        )}
                      </Group>
                    </UnstyledButton>
                  );
                })}
              </Stack>
            </ScrollArea.Autosize>
          )}
        </Box>
      </Collapse>
    </Paper>
  );
}
