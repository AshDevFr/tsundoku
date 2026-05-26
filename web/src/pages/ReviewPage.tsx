import {
  Alert,
  Anchor,
  Badge,
  Box,
  Button,
  Card,
  Center,
  Container,
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
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { useState } from "react";
import {
  useLinkRelease,
  useRejectRelease,
  useRetryRelease,
} from "@/api/mutations";
import {
  type ReleaseDto,
  type ReviewCandidateDto,
  type UnresolvedRelease,
  useProviders,
  useUnresolvedReleases,
} from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";
import { AdminAuthGate } from "@/components/AdminAuthGate";
import { useAdminAuth } from "@/stores/auth";

export function ReviewPage() {
  return (
    <AdminAuthGate>
      <ReviewQueue />
    </AdminAuthGate>
  );
}

function ReviewQueue() {
  const [page, setPage] = useState(1);
  const queue = useUnresolvedReleases(page);
  const clearToken = useAdminAuth((s) => s.clear);

  const total = queue.data?.total ?? 0;
  const pageSize = queue.data?.pageSize ?? 20;
  const totalPages = Math.max(1, Math.ceil(total / pageSize));

  return (
    <Container size="lg" py="lg">
      <Stack gap="lg">
        <Group justify="space-between" align="baseline">
          <Stack gap={2}>
            <Title order={2}>Review queue</Title>
            <Text size="sm" c="dimmed">
              {queue.isLoading
                ? "loading…"
                : `${total.toLocaleString()} release${total === 1 ? "" : "s"} awaiting a decision`}
            </Text>
          </Stack>
          <Tooltip label="Forget the admin token in this browser">
            <Button
              variant="subtle"
              size="xs"
              color="gray"
              onClick={() => clearToken()}
            >
              Sign out
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
            Nothing waiting for review. New unresolved releases will land here
            as the scheduler runs.
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
    </Container>
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
              Link by external ID
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

      <ManualLinkModal
        opened={manualOpen}
        onClose={closeManual}
        releaseId={item.id}
      />
    </Paper>
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
            padding="xs"
            radius="sm"
            data-testid={`candidate-${c.seriesId}`}
          >
            <Group justify="space-between" wrap="nowrap" align="center">
              <Group gap="sm" wrap="nowrap" style={{ minWidth: 0, flex: 1 }}>
                <Box w={36} miw={36} h={48}>
                  <Image
                    src={c.seriesCoverUrl ?? CANDIDATE_PLACEHOLDER}
                    fallbackSrc={CANDIDATE_PLACEHOLDER}
                    alt={c.seriesTitle}
                    radius="sm"
                    h={48}
                    fit="cover"
                  />
                </Box>
                <Stack gap={2} style={{ minWidth: 0, flex: 1 }}>
                  <Text size="sm" fw={500} lineClamp={1} title={c.seriesTitle}>
                    {c.seriesTitle}
                  </Text>
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

function ManualLinkModal({
  opened,
  onClose,
  releaseId,
}: {
  opened: boolean;
  onClose: () => void;
  releaseId: string;
}) {
  const providers = useProviders();
  const link = useLinkRelease();
  const [provider, setProvider] = useState<string | null>(null);
  const [externalId, setExternalId] = useState("");

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

  const submit = () => {
    const trimmedId = externalId.trim();
    if (!effectiveProvider || !trimmedId) return;
    link.mutate(
      {
        releaseId,
        body: { provider: effectiveProvider, externalId: trimmedId },
      },
      {
        onSuccess: () => {
          notifications.show({
            color: "green",
            message: `Linked via ${effectiveProvider}:${trimmedId}`,
          });
          setExternalId("");
          setProvider(null);
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
      title="Link by external ID"
      centered
    >
      <Stack>
        <Text size="sm" c="dimmed">
          Pick a provider and paste its external ID. If no
          <Text span ff="monospace" mx={4}>
            series_external_ids
          </Text>
          row exists yet, the provider's
          <Text span ff="monospace" mx={4}>
            get
          </Text>
          is called to fetch metadata and create the series row.
        </Text>
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
          placeholder="e.g. 1677"
          value={externalId}
          onChange={(e) => setExternalId(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") submit();
          }}
          data-testid="manual-external-id"
        />
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button
            onClick={submit}
            loading={link.isPending}
            disabled={!effectiveProvider || !externalId.trim()}
          >
            Link release
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}
