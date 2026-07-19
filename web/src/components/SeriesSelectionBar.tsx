import {
  Button,
  Checkbox,
  Group,
  Modal,
  Paper,
  Stack,
  Text,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import {
  useBulkRefreshMetadata,
  useBulkSearchReleases,
  useBulkSetWishlisted,
} from "@/api/mutations";

/// Sticky action bar shown while a series selection is non-empty (Feed and
/// Wishlist pages). Modeled on the Review page's selection bar: select-all-
/// on-page, count, Clear, then the bulk actions. Wishlist add/remove are two
/// explicit buttons (set, not toggle); the search action confirms first
/// because it spawns one background walk per selected series.
export function SeriesSelectionBar({
  selectedIds,
  allPageSelected,
  somePageSelected,
  onToggleAll,
  onClear,
  hideAddToWishlist = false,
}: {
  selectedIds: number[];
  allPageSelected: boolean;
  somePageSelected: boolean;
  onToggleAll: () => void;
  onClear: () => void;
  /// The Wishlist page hides "Add to wishlist" — everything there is
  /// already clipped.
  hideAddToWishlist?: boolean;
}) {
  const count = selectedIds.length;
  const wishlist = useBulkSetWishlisted();
  const refresh = useBulkRefreshMetadata();
  const search = useBulkSearchReleases();
  const [confirmOpen, { open: openConfirm, close: closeConfirm }] =
    useDisclosure(false);

  const fail = (title: string) => (e: Error) =>
    notifications.show({ color: "red", title, message: e.message });

  const setWishlisted = (wishlisted: boolean) =>
    wishlist.mutate(
      { ids: selectedIds, wishlisted },
      {
        onSuccess: (d) => {
          notifications.show({
            color: "green",
            message: wishlisted
              ? `Added ${d.updated} series to the wishlist`
              : `Removed ${d.updated} series from the wishlist`,
          });
          onClear();
        },
        onError: fail("Wishlist update failed"),
      },
    );

  const runRefresh = () =>
    refresh.mutate(
      { ids: selectedIds },
      {
        onSuccess: (d) => {
          const reasons = [...new Set(d.skipped.map((s) => s.reason))];
          notifications.show({
            color: d.skipped.length > 0 ? "yellow" : "green",
            message:
              d.skipped.length > 0
                ? `${d.refreshed} refreshed, ${d.skipped.length} skipped: ${reasons.join("; ")}`
                : `${d.refreshed} series refreshed`,
          });
          onClear();
        },
        onError: fail("Metadata refresh failed"),
      },
    );

  const runSearch = () =>
    search.mutate(
      { ids: selectedIds },
      {
        onSuccess: (d) => {
          closeConfirm();
          if (d.skipped) {
            // The whole batch was skipped (a walk is already in flight);
            // keep the selection so the operator can retry as-is.
            notifications.show({
              color: "yellow",
              message: `A "${d.search}" search is already running; try again when it finishes`,
            });
            return;
          }
          notifications.show({
            color: "green",
            message: `Launched ${d.matched} release search${d.matched === 1 ? "" : "es"} on "${d.search}"`,
          });
          onClear();
        },
        onError: (e) => {
          closeConfirm();
          fail("Search launch failed")(e);
        },
      },
    );

  return (
    <Paper
      withBorder
      p="sm"
      pos="sticky"
      top={64}
      style={{ zIndex: 5 }}
      data-testid="series-selection-bar"
    >
      <Group justify="space-between" wrap="wrap" gap="xs">
        <Group gap="sm" wrap="nowrap">
          <Checkbox
            checked={allPageSelected}
            indeterminate={somePageSelected && !allPageSelected}
            onChange={onToggleAll}
            aria-label="Select all on this page"
            data-testid="series-select-page"
          />
          <Text size="sm" fw={500}>
            {count.toLocaleString()} selected
          </Text>
          <Button size="xs" variant="subtle" onClick={onClear}>
            Clear
          </Button>
        </Group>
        <Group gap="xs" wrap="wrap">
          {!hideAddToWishlist && (
            <Button
              size="xs"
              variant="default"
              loading={wishlist.isPending && wishlist.variables?.wishlisted}
              onClick={() => setWishlisted(true)}
              data-testid="bulk-wishlist-add"
            >
              Add to wishlist
            </Button>
          )}
          <Button
            size="xs"
            variant="default"
            loading={
              wishlist.isPending && wishlist.variables?.wishlisted === false
            }
            onClick={() => setWishlisted(false)}
            data-testid="bulk-wishlist-remove"
          >
            Remove from wishlist
          </Button>
          <Button
            size="xs"
            variant="default"
            onClick={openConfirm}
            data-testid="bulk-search"
          >
            Search releases
          </Button>
          <Button
            size="xs"
            variant="default"
            loading={refresh.isPending}
            onClick={runRefresh}
            data-testid="bulk-refresh"
          >
            Refresh metadata
          </Button>
        </Group>
      </Group>

      <Modal
        opened={confirmOpen}
        onClose={closeConfirm}
        title="Launch release searches"
        centered
      >
        <Stack gap="md">
          <Text size="sm">
            Launch {count.toLocaleString()} release search
            {count === 1 ? "" : "es"}? They run one after another in the
            background against the default search entry.
          </Text>
          <Group justify="flex-end" gap="xs">
            <Button variant="default" onClick={closeConfirm}>
              Cancel
            </Button>
            <Button
              loading={search.isPending}
              onClick={runSearch}
              data-testid="bulk-search-confirm"
            >
              Search {count.toLocaleString()}
            </Button>
          </Group>
        </Stack>
      </Modal>
    </Paper>
  );
}
