import {
  Alert,
  Anchor,
  Badge,
  Button,
  Center,
  Group,
  Loader,
  Pagination,
  Paper,
  Select,
  Stack,
  Text,
  TextInput,
  Title,
} from "@mantine/core";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import {
  type ReleaseDto,
  type ReleaseSearchFilters,
  useReleaseSearch,
} from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";
import { formatBytes } from "@/components/admin/format";
import { SentBadge } from "@/components/SendToClientButton";

/// Every resolution status the resolver or the operator can write. Unlike the
/// review queue this page is deliberately unscoped — a `rejected` release has
/// no other surface in the UI, so if it is not reachable here it is not
/// reachable at all.
const STATUS_OPTIONS = [
  { value: "", label: "Any status" },
  { value: "resolved", label: "resolved" },
  { value: "unresolved", label: "unresolved" },
  { value: "ambiguous", label: "ambiguous" },
  { value: "review_pending", label: "review pending" },
  { value: "rejected", label: "rejected" },
  { value: "standalone", label: "standalone (kept)" },
];

const SORT_OPTIONS = [
  { value: "", label: "Newest posted" },
  { value: "observed_desc", label: "Recently discovered" },
  { value: "observed_asc", label: "Least recently discovered" },
  { value: "posted_asc", label: "Oldest posted" },
  { value: "title_asc", label: "Title A→Z" },
  { value: "title_desc", label: "Title Z→A" },
];

const PROVIDER_OPTIONS = [
  { value: "", label: "—" },
  { value: "mangabaka", label: "MangaBaka" },
  { value: "mangaupdates", label: "MangaUpdates" },
  { value: "anilist", label: "AniList" },
  { value: "mal", label: "MyAnimeList" },
  { value: "mangadex", label: "MangaDex" },
  { value: "kitsu", label: "Kitsu" },
  { value: "shikimori", label: "Shikimori" },
  { value: "anime_planet", label: "Anime-Planet" },
  { value: "anime_news_network", label: "Anime News Network" },
];

const STATUS_COLORS: Record<string, string> = {
  resolved: "green",
  unresolved: "gray",
  ambiguous: "yellow",
  review_pending: "orange",
  rejected: "red",
  standalone: "blue",
};

/// Catalog-wide release search: the answer to "I know this release exists
/// upstream — did we ingest it, and where did it go?".
///
/// The review queue can only ever show the three undecided statuses, so a
/// release that resolved to the wrong series, or that was rejected, used to be
/// unreachable from the UI entirely.
export function AdminReleasesPage() {
  // Draft vs applied: the query only runs against submitted values, so typing
  // an id or pasting a URL does not fire a request per keystroke.
  const [draft, setDraft] = useState<ReleaseSearchFilters>({});
  const [applied, setApplied] = useState<ReleaseSearchFilters>({});
  const [page, setPage] = useState(1);

  const search = useReleaseSearch({ ...applied, page });
  const total = search.data?.total ?? 0;
  const pageSize = search.data?.pageSize ?? 20;
  const totalPages = Math.max(1, Math.ceil(total / pageSize));

  const apply = () => {
    setPage(1);
    setApplied(draft);
  };
  const clear = () => {
    setDraft({});
    setApplied({});
    setPage(1);
  };
  const update = (patch: Partial<ReleaseSearchFilters>) =>
    setDraft((d) => ({ ...d, ...patch }));

  return (
    <Stack gap="md">
      <Stack gap={2}>
        <Title order={3}>Releases</Title>
        <Text size="sm" c="dimmed">
          Search every discovered release, at any status. Paste a post URL to
          check whether it was ingested.
        </Text>
      </Stack>

      <Paper withBorder radius="md" p="md">
        <Stack gap="sm">
          <TextInput
            label="Search"
            placeholder="Blacksmith v01, or https://nyaa.si/view/1997229"
            description="Words match in any order. A post URL or bare post id jumps straight to that release."
            value={draft.q ?? ""}
            onChange={(e) => update({ q: e.currentTarget.value })}
            onKeyDown={(e) => {
              if (e.key === "Enter") apply();
            }}
            data-testid="releases-q"
          />
          <Group grow align="flex-end">
            <Select
              label="Status"
              data={STATUS_OPTIONS}
              value={draft.status ?? ""}
              onChange={(v) => update({ status: v ?? "" })}
              allowDeselect={false}
              data-testid="releases-status"
            />
            <TextInput
              label="Source"
              placeholder="nyaa-1r0n"
              value={draft.sourceName ?? ""}
              onChange={(e) => update({ sourceName: e.currentTarget.value })}
              data-testid="releases-source"
            />
            <TextInput
              label="Format"
              placeholder="cbz"
              value={draft.format ?? ""}
              onChange={(e) => update({ format: e.currentTarget.value })}
              data-testid="releases-format"
            />
            <Select
              label="Sort"
              data={SORT_OPTIONS}
              value={draft.sort ?? ""}
              onChange={(v) => update({ sort: v ?? "" })}
              allowDeselect={false}
              data-testid="releases-sort"
            />
          </Group>
          <Group grow align="flex-end">
            <Select
              label="Provider"
              description="With an ID, lists every release linked to that series."
              data={PROVIDER_OPTIONS}
              value={draft.provider ?? ""}
              onChange={(v) => update({ provider: v ?? "" })}
              allowDeselect={false}
              data-testid="releases-provider"
            />
            <TextInput
              label="Provider ID"
              placeholder="6734"
              value={draft.externalId ?? ""}
              onChange={(e) => update({ externalId: e.currentTarget.value })}
              onKeyDown={(e) => {
                if (e.key === "Enter") apply();
              }}
              data-testid="releases-external-id"
            />
          </Group>
          <Group justify="flex-end">
            <Button variant="default" onClick={clear}>
              Clear
            </Button>
            <Button onClick={apply} data-testid="releases-apply">
              Search
            </Button>
          </Group>
        </Stack>
      </Paper>

      <Text size="sm" c="dimmed">
        {search.isLoading
          ? "loading…"
          : `${total.toLocaleString()} release${total === 1 ? "" : "s"}`}
      </Text>

      {search.isError && (
        <Alert color="red" title="Failed to load releases">
          {(search.error as Error)?.message ?? "Unknown error"}
        </Alert>
      )}

      {search.isLoading && !search.data && (
        <Center py="xl">
          <Loader />
        </Center>
      )}

      {search.data && search.data.items.length === 0 && (
        <Alert color="gray" title="Nothing matched">
          No release matches those filters. If you pasted a post URL and
          expected a hit, the release was never ingested — check the source's
          poll history under Sources.
        </Alert>
      )}

      {search.data && search.data.items.length > 0 && (
        <Stack gap="sm" data-testid="releases-results">
          {search.data.items.map((release) => (
            <ReleaseRow key={release.id} release={release} />
          ))}
        </Stack>
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
    </Stack>
  );
}

function ReleaseRow({ release }: { release: ReleaseDto }) {
  return (
    <Paper
      withBorder
      radius="md"
      p="sm"
      data-testid={`release-row-${release.id}`}
    >
      <Stack gap={4}>
        <Anchor
          href={release.link}
          target="_blank"
          rel="noreferrer noopener"
          size="sm"
          fw={600}
          lineClamp={2}
          title={release.title}
        >
          {release.title}
        </Anchor>
        <Group gap={6} wrap="wrap">
          <Badge
            size="xs"
            variant="light"
            color={STATUS_COLORS[release.resolutionStatus] ?? "gray"}
          >
            {release.resolutionStatus}
          </Badge>
          {/* The whole point of the page: where did it go? */}
          {typeof release.seriesId === "number" ? (
            // `renderRoot` instead of `component={Link}`: Mantine's polymorphic
            // prop erases the Link generics, so typed `params` won't compile
            // through it.
            <Anchor
              size="xs"
              renderRoot={(props) => (
                <Link
                  to="/series/$id"
                  params={{ id: String(release.seriesId) }}
                  {...props}
                />
              )}
            >
              series #{release.seriesId}
            </Anchor>
          ) : (
            <Text size="xs" c="dimmed">
              no series
            </Text>
          )}
          <Badge size="xs" color="indigo" variant="light">
            {release.sourceKind}:{release.sourceName}
          </Badge>
          {release.formats.map((f) => (
            <Badge key={f} size="xs" variant="outline">
              {f}
            </Badge>
          ))}
          {typeof release.sizeBytes === "number" && (
            <Text size="xs" c="dimmed">
              {formatBytes(release.sizeBytes)}
            </Text>
          )}
          <Text size="xs" c="dimmed" title={formatAbsolute(release.postedAt)}>
            posted {formatRelative(release.postedAt)}
          </Text>
          <Text size="xs" c="dimmed" title={formatAbsolute(release.observedAt)}>
            found {formatRelative(release.observedAt)}
          </Text>
          <SentBadge release={release} />
        </Group>
      </Stack>
    </Paper>
  );
}
