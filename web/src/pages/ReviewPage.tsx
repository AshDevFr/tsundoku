import {
  Alert,
  Anchor,
  Badge,
  Box,
  Button,
  Card,
  Center,
  Collapse,
  Group,
  Image,
  Loader,
  Modal,
  Pagination,
  Paper,
  Select,
  Stack,
  Text,
  TextInput,
  Title,
  Tooltip,
  Typography,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import rehypeSanitize from "rehype-sanitize";
import remarkGfm from "remark-gfm";
import {
  useLinkRelease,
  useRejectRelease,
  useRetryAllReleases,
  useRetryRelease,
} from "@/api/mutations";
import {
  type ProviderSearchHit,
  type ReleaseDto,
  type ReviewCandidateDto,
  type UnresolvedRelease,
  useProviderSearch,
  useProviders,
  useUnresolvedReleases,
} from "@/api/queries";
import { formatAbsolute, formatRelative, providerUrl } from "@/api/utils";

export function ReviewPage() {
  const [page, setPage] = useState(1);
  const queue = useUnresolvedReleases(page);
  const retryAll = useRetryAllReleases();

  const total = queue.data?.total ?? 0;
  const pageSize = queue.data?.pageSize ?? 20;
  const totalPages = Math.max(1, Math.ceil(total / pageSize));

  const handleRetryAll = () => {
    retryAll.mutate(undefined, {
      onSuccess: (data) => {
        if (data?.skipped) {
          notifications.show({
            color: "gray",
            message: "Retry already in progress",
          });
        } else {
          notifications.show({
            color: "blue",
            message: "Re-running resolver on the review queue",
          });
        }
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Retry all failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Stack gap="md">
      <Group justify="space-between" align="baseline" wrap="wrap">
        <Stack gap={2}>
          <Title order={3}>Review queue</Title>
          <Text size="sm" c="dimmed">
            {queue.isLoading
              ? "loading…"
              : `${total.toLocaleString()} release${total === 1 ? "" : "s"} awaiting a decision`}
          </Text>
        </Stack>
        <Tooltip label="Re-run the resolver against every release currently in this queue">
          <Button
            variant="light"
            size="xs"
            onClick={handleRetryAll}
            loading={retryAll.isPending}
            disabled={total === 0}
            data-testid="retry-all-button"
          >
            Retry all
          </Button>
        </Tooltip>
      </Group>

      {queue.isError && (
        <Alert color="red" title="Failed to load review queue">
          {(queue.error as Error)?.message ?? "Unknown error"}
        </Alert>
      )}

      {queue.isLoading && !queue.data && (
        <Center py="xl">
          <Loader />
        </Center>
      )}

      {queue.data && queue.data.items.length === 0 && (
        <Alert color="green" title="Inbox zero">
          Nothing waiting for review. New unresolved releases will land here as
          the scheduler runs.
        </Alert>
      )}

      {queue.data && queue.data.items.length > 0 && (
        <Stack gap="md">
          {queue.data.items.map((item) => (
            <ReviewCard key={item.id} item={item} />
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

function ReviewCard({ item }: { item: UnresolvedRelease }) {
  const link = useLinkRelease();
  const reject = useRejectRelease();
  const retry = useRetryRelease();
  const [manualOpen, { open: openManual, close: closeManual }] =
    useDisclosure(false);

  const busy = link.isPending || reject.isPending || retry.isPending;

  const handleLinkCandidate = (candidate: ReviewCandidateDto) => {
    link.mutate(
      { releaseId: item.id, body: { seriesId: candidate.seriesId } },
      {
        onSuccess: () => {
          notifications.show({
            color: "green",
            message: `Linked to "${candidate.seriesTitle}"`,
          });
        },
        onError: (e) => {
          notifications.show({
            color: "red",
            title: "Link failed",
            message: (e as Error).message,
          });
        },
      },
    );
  };

  const handleReject = () => {
    reject.mutate(item.id, {
      onSuccess: () =>
        notifications.show({ color: "gray", message: "Release rejected" }),
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Reject failed",
          message: (e as Error).message,
        }),
    });
  };

  const handleRetry = () => {
    retry.mutate(item.id, {
      onSuccess: () =>
        notifications.show({ color: "blue", message: "Re-running resolver" }),
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Retry failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Paper withBorder radius="md" p="md" data-testid={`review-card-${item.id}`}>
      <Stack gap="sm">
        <ReleaseHeader release={item} />
        <ExtractedLinks links={item.extractedLinks} />
        <DescriptionBlock body={item.descriptionHtml} />
        <CleanupTrail
          queries={item.searchQueries}
          rules={item.cleanupRulesApplied}
        />
        <CandidateList
          candidates={item.candidates}
          disabled={busy}
          onPick={handleLinkCandidate}
        />
        <Group justify="space-between" wrap="wrap" gap="xs">
          <Group gap="xs">
            <Button
              variant="light"
              size="xs"
              onClick={openManual}
              disabled={busy}
            >
              Search provider
            </Button>
          </Group>
          <Group gap="xs">
            <Button
              variant="subtle"
              color="gray"
              size="xs"
              onClick={handleRetry}
              loading={retry.isPending}
              disabled={link.isPending || reject.isPending}
            >
              Retry
            </Button>
            <Button
              variant="subtle"
              color="red"
              size="xs"
              onClick={handleReject}
              loading={reject.isPending}
              disabled={link.isPending || retry.isPending}
            >
              Reject
            </Button>
          </Group>
        </Group>
      </Stack>

      <ProviderSearchModal
        opened={manualOpen}
        onClose={closeManual}
        releaseId={item.id}
        seedQuery={item.searchQueries[0] ?? item.title}
      />
    </Paper>
  );
}

/// Diagnostic strip: shows the cleaned primary search query (with any
/// alternates as small chips) and the rule names that fired during
/// cleanup. Surfaces "what surgery happened" without expanding to a
/// debug pane.
function CleanupTrail({
  queries,
  rules,
}: {
  queries: string[];
  rules: string[];
}) {
  if (queries.length === 0 && rules.length === 0) {
    return null;
  }
  return (
    <Stack gap={4} data-testid="cleanup-trail">
      {queries.length > 0 && (
        <Group gap={6} wrap="wrap" align="center">
          <Text size="xs" c="dimmed" tt="uppercase" fw={500}>
            {queries.length > 1 ? "searched (any)" : "searched"}
          </Text>
          {/* Every query is searched independently; the resolver keeps
              the best match across all of them. Show them all so the
              operator can see exactly what was tried. */}
          {queries.map((q) => (
            <Text key={q} size="xs" ff="monospace">
              “{q}”
            </Text>
          ))}
        </Group>
      )}
      {rules.length > 0 && (
        <Group gap={4} wrap="wrap">
          {rules.map((r) => (
            <Badge
              key={r}
              size="xs"
              variant="outline"
              color="grape"
              ff="monospace"
            >
              {r}
            </Badge>
          ))}
        </Group>
      )}
    </Stack>
  );
}

/// Provider links the source scraped from the release description.
/// Shown above the candidate list so the operator can verify a hand-pasted
/// MangaUpdates / AniList / MAL / MangaDex link without leaving the queue.
function ExtractedLinks({
  links,
}: {
  links: UnresolvedRelease["extractedLinks"];
}) {
  if (!links) {
    return null;
  }
  const entries: { provider: string; label: string; href: string }[] = [
    ...(links.mangaupdates
      ? [
          {
            provider: "mangaupdates",
            label: "MangaUpdates",
            href: links.mangaupdates,
          },
        ]
      : []),
    ...(links.anilist
      ? [{ provider: "anilist", label: "AniList", href: links.anilist }]
      : []),
    ...(links.mal ? [{ provider: "mal", label: "MAL", href: links.mal }] : []),
    ...(links.mangadex
      ? [
          {
            provider: "mangadex",
            label: "MangaDex",
            href: links.mangadex,
          },
        ]
      : []),
  ];
  if (entries.length === 0) {
    return null;
  }
  return (
    <Group gap={6} wrap="wrap" data-testid="extracted-links">
      <Text size="xs" c="dimmed" tt="uppercase" fw={500}>
        links
      </Text>
      {entries.map((e) => (
        <Anchor
          key={e.provider}
          href={e.href}
          target="_blank"
          rel="noreferrer noopener"
          size="xs"
        >
          <Badge
            size="sm"
            variant="light"
            color="teal"
            style={{ cursor: "pointer" }}
          >
            {e.label}
          </Badge>
        </Anchor>
      ))}
    </Group>
  );
}

/// Collapsible markdown rendering of the post description. Nyaa uploaders
/// publish bodies in markdown (the `markdown-text` class on the post page);
/// we render with sanitization and GFM tables so the queue card matches
/// what the operator would see on Nyaa.
function DescriptionBlock({ body }: { body: string | null | undefined }) {
  const [opened, { toggle }] = useDisclosure(false);
  const trimmed = body?.trim();
  if (!trimmed) {
    return null;
  }
  return (
    <Box data-testid="description-block">
      <Group gap={6} wrap="wrap" mb={opened ? 4 : 0}>
        <Text size="xs" c="dimmed" tt="uppercase" fw={500}>
          description
        </Text>
        <Anchor
          component="button"
          type="button"
          size="xs"
          onClick={toggle}
          aria-expanded={opened}
        >
          {opened ? "hide" : "show"}
        </Anchor>
      </Group>
      <Collapse expanded={opened}>
        <Paper
          withBorder
          radius="sm"
          p="sm"
          bg="var(--mantine-color-default-hover)"
        >
          <Typography fz="sm">
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              rehypePlugins={[rehypeSanitize]}
              components={{
                a: ({ href, children }) => (
                  <a href={href} target="_blank" rel="noreferrer noopener">
                    {children}
                  </a>
                ),
              }}
            >
              {trimmed}
            </ReactMarkdown>
          </Typography>
        </Paper>
      </Collapse>
    </Box>
  );
}

function ReleaseHeader({ release }: { release: ReleaseDto }) {
  return (
    <Stack gap={4}>
      <Group justify="space-between" align="flex-start" wrap="nowrap">
        <Stack gap={2} style={{ flex: 1, minWidth: 0 }}>
          <Anchor
            href={release.link}
            target="_blank"
            rel="noreferrer noopener"
            size="md"
            fw={600}
            lineClamp={2}
            title={release.title}
          >
            {release.title}
          </Anchor>
          <Group gap={6} wrap="wrap">
            <Badge size="xs" color="indigo" variant="light">
              {release.sourceKind}:{release.sourceName}
            </Badge>
            {release.formats.map((f) => (
              <Badge key={f} size="xs" variant="outline">
                {f}
              </Badge>
            ))}
            <Badge size="xs" color="orange" variant="light">
              {release.resolutionStatus}
            </Badge>
            <Text size="xs" c="dimmed" title={formatAbsolute(release.postedAt)}>
              posted {formatRelative(release.postedAt)}
            </Text>
            {release.resolutionAttempts > 0 && (
              <Text size="xs" c="dimmed">
                {release.resolutionAttempts} attempt
                {release.resolutionAttempts === 1 ? "" : "s"}
              </Text>
            )}
          </Group>
        </Stack>
        <Group gap={6} wrap="nowrap">
          {release.magnet && (
            <Anchor href={release.magnet} size="xs" rel="noreferrer">
              magnet
            </Anchor>
          )}
          {release.torrentUrl && (
            <Anchor
              href={release.torrentUrl}
              size="xs"
              target="_blank"
              rel="noreferrer noopener"
            >
              .torrent
            </Anchor>
          )}
        </Group>
      </Group>
    </Stack>
  );
}

const CANDIDATE_PLACEHOLDER =
  "data:image/svg+xml;utf8,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 3 4%22%3E%3Crect width=%223%22 height=%224%22 fill=%22%23ced4da%22/%3E%3C/svg%3E";

function CandidateList({
  candidates,
  disabled,
  onPick,
}: {
  candidates: ReviewCandidateDto[];
  disabled: boolean;
  onPick: (c: ReviewCandidateDto) => void;
}) {
  if (candidates.length === 0) {
    return (
      <Text size="sm" c="dimmed">
        No candidate matches — link manually or reject.
      </Text>
    );
  }
  return (
    <Stack gap={6}>
      <Text size="xs" fw={500} c="dimmed" tt="uppercase">
        Candidates
      </Text>
      <Stack gap={6}>
        {candidates.map((c) => (
          <Card
            key={c.seriesId}
            withBorder
            padding="sm"
            radius="sm"
            data-testid={`candidate-${c.seriesId}`}
          >
            <Group justify="space-between" wrap="nowrap" align="flex-start">
              <Group
                gap="sm"
                wrap="nowrap"
                align="flex-start"
                style={{ minWidth: 0, flex: 1 }}
              >
                <Box w={56} miw={56} h={76}>
                  <Image
                    src={c.seriesCoverUrl ?? CANDIDATE_PLACEHOLDER}
                    fallbackSrc={CANDIDATE_PLACEHOLDER}
                    alt={c.seriesTitle}
                    radius="sm"
                    h={76}
                    fit="cover"
                  />
                </Box>
                <Stack gap={4} style={{ minWidth: 0, flex: 1 }}>
                  <Group gap={6} wrap="nowrap" style={{ minWidth: 0 }}>
                    <Text
                      size="md"
                      fw={600}
                      lineClamp={2}
                      title={c.seriesTitle}
                      style={{ minWidth: 0, flex: 1 }}
                    >
                      {c.seriesTitle}
                    </Text>
                    {c.provider &&
                      c.externalId &&
                      (() => {
                        const href = providerUrl(c.provider, c.externalId);
                        return href ? (
                          <Anchor
                            href={href}
                            target="_blank"
                            rel="noreferrer noopener"
                            size="xs"
                            title="Open on provider"
                          >
                            view ↗
                          </Anchor>
                        ) : null;
                      })()}
                  </Group>
                  {c.alternateTitles.length > 0 && (
                    <Text
                      size="xs"
                      c="dimmed"
                      style={{ wordBreak: "break-word" }}
                    >
                      {c.alternateTitles.join(" / ")}
                    </Text>
                  )}
                  <Group gap={6}>
                    <Badge size="xs" variant="default">
                      score {c.score.toFixed(2)}
                    </Badge>
                    {c.reason && (
                      <Text size="xs" c="dimmed">
                        {c.reason}
                      </Text>
                    )}
                  </Group>
                </Stack>
              </Group>
              <Button
                size="xs"
                variant="light"
                onClick={() => onPick(c)}
                disabled={disabled}
                data-testid={`link-candidate-${c.seriesId}`}
              >
                Link
              </Button>
            </Group>
          </Card>
        ))}
      </Stack>
    </Stack>
  );
}

/// Modal for linking a review-queue release to a provider series. Two
/// paths share one UI:
///
/// - Paste an external ID → exact lookup, single result, one click.
/// - Type a title → debounced search, scrollable result list,
///   click-to-link.
///
/// External ID takes priority when both are filled; the helper text
/// makes that explicit.
function ProviderSearchModal({
  opened,
  onClose,
  releaseId,
  seedQuery,
}: {
  opened: boolean;
  onClose: () => void;
  releaseId: string;
  seedQuery: string;
}) {
  const providers = useProviders();
  const link = useLinkRelease();
  const [provider, setProvider] = useState<string | null>(null);
  const [title, setTitle] = useState(seedQuery);
  const [externalId, setExternalId] = useState("");
  // Debounce the title input so each keystroke doesn't fire a search.
  const [debouncedTitle, setDebouncedTitle] = useState(seedQuery);

  // Reset state when the modal opens against a new release.
  useEffect(() => {
    if (opened) {
      setTitle(seedQuery);
      setDebouncedTitle(seedQuery);
      setExternalId("");
      setProvider(null);
    }
  }, [opened, seedQuery]);

  useEffect(() => {
    const handle = window.setTimeout(() => setDebouncedTitle(title), 300);
    return () => window.clearTimeout(handle);
  }, [title]);

  const options =
    providers.data?.items.map((p) => ({
      value: p.id,
      label: p.active ? `${p.displayName} (active)` : p.displayName,
    })) ?? [];

  const activeId =
    providers.data?.items.find((p) => p.active)?.id ??
    options[0]?.value ??
    null;

  const effectiveProvider = provider ?? activeId;

  const search = useProviderSearch({
    providerId: effectiveProvider,
    q: debouncedTitle,
    externalId,
    enabled: opened,
  });

  const handleLink = (chosenExternalId: string, displayLabel: string) => {
    if (!effectiveProvider) return;
    link.mutate(
      {
        releaseId,
        body: { provider: effectiveProvider, externalId: chosenExternalId },
      },
      {
        onSuccess: () => {
          notifications.show({
            color: "green",
            message: `Linked to ${displayLabel}`,
          });
          onClose();
        },
        onError: (e) => {
          notifications.show({
            color: "red",
            title: "Link failed",
            message: (e as Error).message,
          });
        },
      },
    );
  };

  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title="Search provider"
      size="lg"
      centered
    >
      <Stack gap="md">
        <Select
          label="Provider"
          data={options}
          value={effectiveProvider}
          onChange={setProvider}
          allowDeselect={false}
          searchable={options.length > 5}
        />
        <TextInput
          label="External ID"
          description="Paste a provider ID to look up directly (takes priority over title)"
          placeholder="e.g. 12345"
          value={externalId}
          onChange={(e) => setExternalId(e.currentTarget.value)}
          data-testid="search-external-id"
        />
        <TextInput
          label="Title"
          placeholder="Search by series title"
          value={title}
          onChange={(e) => setTitle(e.currentTarget.value)}
          disabled={externalId.trim().length > 0}
          data-testid="search-title"
        />

        <SearchResults
          provider={effectiveProvider}
          hits={search.data?.hits ?? []}
          loading={search.isFetching}
          enabled={Boolean(
            effectiveProvider && (debouncedTitle.trim() || externalId.trim()),
          )}
          disabled={link.isPending}
          onPick={handleLink}
        />

        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Close
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}

function SearchResults({
  provider,
  hits,
  loading,
  enabled,
  disabled,
  onPick,
}: {
  provider: string | null;
  hits: ProviderSearchHit[];
  loading: boolean;
  enabled: boolean;
  disabled: boolean;
  onPick: (externalId: string, displayLabel: string) => void;
}) {
  if (!enabled) {
    return (
      <Text size="xs" c="dimmed">
        Enter a title or external ID above to search.
      </Text>
    );
  }
  if (loading && hits.length === 0) {
    return (
      <Center py="md">
        <Loader size="sm" />
      </Center>
    );
  }
  if (hits.length === 0) {
    return (
      <Text size="xs" c="dimmed">
        No results.
      </Text>
    );
  }
  return (
    <Stack gap={6} data-testid="search-results">
      <Text size="xs" fw={500} c="dimmed" tt="uppercase">
        {hits.length} result{hits.length === 1 ? "" : "s"}
      </Text>
      <Stack gap={6} mah={400} style={{ overflowY: "auto" }}>
        {hits.map((h) => (
          <Card
            key={`${h.externalId}-${h.title}`}
            withBorder
            padding="xs"
            radius="sm"
            data-testid={`search-hit-${h.externalId}`}
            style={{ flexShrink: 0 }}
          >
            <Group justify="space-between" wrap="nowrap" align="center">
              <Group gap="sm" wrap="nowrap" style={{ minWidth: 0, flex: 1 }}>
                <Box w={42} miw={42} h={56}>
                  <Image
                    src={h.coverUrl ?? CANDIDATE_PLACEHOLDER}
                    fallbackSrc={CANDIDATE_PLACEHOLDER}
                    alt={h.title}
                    radius="sm"
                    h={56}
                    fit="cover"
                  />
                </Box>
                <Stack gap={2} style={{ minWidth: 0, flex: 1 }}>
                  <Group gap={6} wrap="nowrap" style={{ minWidth: 0 }}>
                    <Text
                      size="sm"
                      fw={500}
                      lineClamp={1}
                      title={h.title}
                      style={{ minWidth: 0, flex: 1 }}
                    >
                      {h.title}
                    </Text>
                    {provider &&
                      (() => {
                        const href = providerUrl(provider, h.externalId);
                        return href ? (
                          <Anchor
                            href={href}
                            target="_blank"
                            rel="noreferrer noopener"
                            size="xs"
                            title="Open on provider"
                          >
                            view ↗
                          </Anchor>
                        ) : null;
                      })()}
                  </Group>
                  {h.nativeTitle && (
                    <Text size="xs" c="dimmed" lineClamp={1}>
                      {h.nativeTitle}
                    </Text>
                  )}
                  <Group gap={6} wrap="wrap">
                    <Badge size="xs" variant="default">
                      score {h.score.toFixed(2)}
                    </Badge>
                    {h.year && (
                      <Badge size="xs" variant="light" color="gray">
                        {h.year}
                      </Badge>
                    )}
                    {h.kind && (
                      <Badge size="xs" variant="light" color="indigo">
                        {h.kind}
                      </Badge>
                    )}
                    {h.status && (
                      <Badge size="xs" variant="light" color="teal">
                        {h.status}
                      </Badge>
                    )}
                  </Group>
                </Stack>
              </Group>
              <Button
                size="xs"
                variant="light"
                onClick={() => onPick(h.externalId, h.title)}
                disabled={disabled}
                data-testid={`link-hit-${h.externalId}`}
              >
                Link
              </Button>
            </Group>
          </Card>
        ))}
      </Stack>
    </Stack>
  );
}
