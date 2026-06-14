import {
  Alert,
  Box,
  Button,
  Center,
  Group,
  Loader,
  Modal,
  Pagination,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { useState } from "react";
import { useCreateSeriesFromProvider } from "@/api/mutations";
import { useSeriesList } from "@/api/queries";
import { ProviderSearchControls } from "@/components/ReleaseLinking";
import { SeriesCard } from "@/components/SeriesCard";
import { DEFAULT_PAGE_SIZE } from "@/stores/uiPrefs";

/// Admin-only curated "download later" list: the series the operator has
/// clipped, newest clip first. Reuses the feed's `SeriesCard` (so owned /
/// Codex / coverage context shows), and offers an "Add from MangaBaka" search
/// for series with no discovered release yet.
export function WishlistPage() {
  const [page, setPage] = useState(1);
  const [addOpen, { open: openAdd, close: closeAdd }] = useDisclosure(false);

  const list = useSeriesList({
    wishlisted: true,
    sort: "wishlisted_at",
    order: "desc",
    page,
    pageSize: DEFAULT_PAGE_SIZE,
  });

  const total = list.data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / DEFAULT_PAGE_SIZE));
  const codexSynced = Boolean(list.data?.codexSyncedAt);

  return (
    <Stack gap="md">
      <Group justify="space-between" align="flex-end" wrap="wrap">
        <Stack gap={2}>
          <Title order={3}>Wishlist</Title>
          <Text size="sm" c="dimmed">
            {list.isLoading
              ? "loading…"
              : `${total.toLocaleString()} clipped series — your "download later" list`}
          </Text>
        </Stack>
        <Button onClick={openAdd} data-testid="wishlist-add-open">
          Add from MangaBaka
        </Button>
      </Group>

      {list.isError && (
        <Alert color="red" title="Failed to load wishlist">
          {(list.error as Error)?.message ?? "Unknown error"}
        </Alert>
      )}

      {list.isLoading && !list.data && (
        <Center py="xl">
          <Loader />
        </Center>
      )}

      {list.data && list.data.items.length === 0 && (
        <Alert color="gray" title="Nothing on the wishlist yet">
          Clip a series with the ★ on its card or detail page, or use “Add from
          MangaBaka” to track one that has no release yet.
        </Alert>
      )}

      {list.data && list.data.items.length > 0 && (
        <Box
          data-testid="wishlist-card-grid"
          style={{
            display: "grid",
            gap: "var(--mantine-spacing-md)",
            gridTemplateColumns: "repeat(auto-fill, minmax(175px, 1fr))",
          }}
        >
          {list.data.items.map((s) => (
            <SeriesCard key={s.id} series={s} codexSynced={codexSynced} />
          ))}
        </Box>
      )}

      {totalPages > 1 && (
        <Center>
          <Pagination
            value={page}
            onChange={setPage}
            total={totalPages}
            size="sm"
          />
        </Center>
      )}

      <Modal
        opened={addOpen}
        onClose={closeAdd}
        title="Add a series from MangaBaka"
        size="lg"
        centered
      >
        <WishlistAddSearch onAdded={closeAdd} />
      </Modal>
    </Stack>
  );
}

/// Provider search wired to create + wishlist the picked series. Reuses the
/// same search surface as the review link flow; on pick it materializes the
/// series (provider-backed) and clips it to the wishlist in one request.
function WishlistAddSearch({ onAdded }: { onAdded: () => void }) {
  const create = useCreateSeriesFromProvider();

  const handlePick = (provider: string, externalId: string, label: string) => {
    create.mutate(
      { provider, externalId, wishlist: true },
      {
        onSuccess: () => {
          notifications.show({
            color: "green",
            message: `Added "${label}" to the wishlist`,
          });
          onAdded();
        },
        onError: (e) => {
          notifications.show({
            color: "red",
            title: "Add failed",
            message: (e as Error).message,
          });
        },
      },
    );
  };

  return (
    <ProviderSearchControls
      seedQuery=""
      disabled={create.isPending}
      actionLabel="Add"
      onPick={handlePick}
    />
  );
}
