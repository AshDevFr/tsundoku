import {
  Alert,
  Box,
  Center,
  Container,
  Grid,
  Group,
  Loader,
  Pagination,
  SegmentedControl,
  Select,
  SimpleGrid,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { useNavigate } from "@tanstack/react-router";
import { useSeriesList } from "@/api/queries";
import { FilterPanel } from "@/components/FilterPanel";
import { SeriesCard } from "@/components/SeriesCard";
import { SeriesListRow } from "@/components/SeriesListRow";
import { feedRoute } from "@/router";
import {
  DEFAULT_PAGE_SIZE,
  type FilterSearch,
  PAGE_SIZE_OPTIONS,
} from "@/stores/filters";

export function FeedPage() {
  const search = feedRoute.useSearch();
  const navigate = useNavigate({ from: feedRoute.fullPath });

  const setSearch = (next: FilterSearch) =>
    navigate({ search: () => next, replace: false });

  const list = useSeriesList(search);
  const total = list.data?.total ?? 0;
  const pageSize = list.data?.pageSize ?? DEFAULT_PAGE_SIZE;
  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  const view: "card" | "list" = search.view === "list" ? "list" : "card";

  return (
    <Container size="xl" py="lg">
      <Grid gap="lg">
        <Grid.Col span={{ base: 12, sm: 4, md: 3 }}>
          <Box pos={{ base: "static", sm: "sticky" }} top={72}>
            <FilterPanel search={search} onChange={setSearch} />
          </Box>
        </Grid.Col>

        <Grid.Col span={{ base: 12, sm: 8, md: 9 }}>
          <Stack gap="md">
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
                  value={String(search.pageSize ?? DEFAULT_PAGE_SIZE)}
                  onChange={(v) => {
                    const next = Number(v);
                    setSearch({
                      ...search,
                      // Keep the URL clean: only persist a non-default size.
                      pageSize: next === DEFAULT_PAGE_SIZE ? undefined : next,
                      page: 1,
                    });
                  }}
                  allowDeselect={false}
                  data-testid="feed-page-size"
                />
                <SegmentedControl
                  size="xs"
                  value={view}
                  onChange={(v) =>
                    setSearch({
                      ...search,
                      view: v === "list" ? "list" : "card",
                    })
                  }
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

            {list.data && list.data.items.length > 0 && view === "card" && (
              <SimpleGrid
                cols={{ base: 2, xs: 3, sm: 3, md: 4, lg: 5 }}
                spacing="md"
                verticalSpacing="md"
              >
                {list.data.items.map((s) => (
                  <SeriesCard key={s.id} series={s} />
                ))}
              </SimpleGrid>
            )}

            {list.data && list.data.items.length > 0 && view === "list" && (
              <Stack gap="xs" data-testid="feed-list-view">
                {list.data.items.map((s) => (
                  <SeriesListRow key={s.id} series={s} />
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
        </Grid.Col>
      </Grid>
    </Container>
  );
}
