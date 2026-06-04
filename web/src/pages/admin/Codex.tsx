import {
  Alert,
  Badge,
  Button,
  Group,
  Loader,
  Paper,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { useJobEventFor } from "@/api/jobEventsContext";
import { useCodexRefresh, useTestCodex } from "@/api/mutations";
import {
  type CodexStatusDto,
  type HealthCheckDto,
  useCodexStatus,
} from "@/api/queries";
import { formatAbsolute, formatRelative } from "@/api/utils";

/// First Codex release that ships the `series/external-index` endpoint the
/// presence sync depends on. Older servers can't be swept.
const MIN_CODEX_VERSION = "1.32.0";

/// True when `version` is a parseable semver strictly below [`MIN_CODEX_VERSION`].
/// Returns false when the version is missing or unparseable — we only warn when
/// we're sure it's too old, never on a hunch.
export function codexVersionOutdated(
  version: string | null | undefined,
): boolean {
  const parse = (v: string): number[] | null => {
    const parts = v
      .trim()
      .replace(/^v/i, "")
      .split(".")
      .map((p) => Number.parseInt(p, 10));
    return parts.length > 0 && parts.every((n) => Number.isFinite(n))
      ? parts
      : null;
  };
  const a = version ? parse(version) : null;
  const b = parse(MIN_CODEX_VERSION);
  if (!a || !b) return false;
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i] ?? 0;
    const y = b[i] ?? 0;
    if (x < y) return true;
    if (x > y) return false;
  }
  return false;
}

/// Mantine color + human label for each Codex `auth_state`. Names the exact
/// operator fix for the two failure modes (wrong key vs under-scoped key).
const AUTH_STATE_META: Record<string, { color: string; label: string }> = {
  ok: { color: "green", label: "Connected" },
  unauthorized: { color: "red", label: "API key rejected (401)" },
  forbidden: { color: "red", label: "API key lacks series:read (403)" },
  unknown: { color: "gray", label: "Not checked yet" },
};

/// Dedicated admin page for the Codex presence integration: connection health,
/// reachability history, a manual connection test, and a manual sweep trigger.
/// Split off the Maintenance page so the growing check history has room.
export function AdminCodexPage() {
  const status = useCodexStatus();
  const refresh = useCodexRefresh();
  const test = useTestCodex();
  const qc = useQueryClient();
  // The sweep runs async after the trigger returns, so invalidating in the
  // mutation's onSuccess refetches the *pre-sweep* row. Instead, refetch when
  // the SSE stream reports the codex job finished, so the panel reflects the
  // true outcome (error cleared on success, or the new error on failure).
  const codexEvent = useJobEventFor("codex", "codex");
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-run when a new finished frame lands (keyed by `at`).
  useEffect(() => {
    if (codexEvent?.phase === "finished") {
      qc.invalidateQueries({ queryKey: ["codex-status"] });
      qc.invalidateQueries({ queryKey: ["series-list"] });
    }
  }, [codexEvent?.phase, codexEvent?.at, qc]);

  const handleRefresh = () => {
    refresh.mutate(undefined, {
      onSuccess: (data) => {
        notifications.show({
          color: data?.triggered ? "blue" : "gray",
          title: data?.triggered
            ? "Codex sync triggered"
            : "Sync already running",
          message: data?.triggered
            ? "A presence sweep is running; ownership badges refresh when it finishes."
            : "A sweep is already in flight; this request was a no-op.",
        });
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Codex refresh failed",
          message: (e as Error).message,
        }),
    });
  };

  const handleTest = () => {
    test.mutate(undefined, {
      onSuccess: (data) => {
        notifications.show({
          color: data?.reachable ? "teal" : "red",
          message: data?.reachable
            ? "Codex reachable"
            : `Unreachable: ${data?.lastError ?? "unknown error"}`,
        });
      },
      onError: (e) =>
        notifications.show({
          color: "red",
          title: "Codex connection test failed",
          message: (e as Error).message,
        }),
    });
  };

  return (
    <Stack gap="md">
      <Stack gap={2}>
        <Title order={3}>Codex integration</Title>
        <Text size="xs" c="dimmed">
          Connection health for the Codex presence sync. Ownership badges on the
          feed come from a periodic sweep; trigger one manually here. Configured
          from the <code>[codex]</code> block.
        </Text>
      </Stack>

      {status.isLoading && <Loader size="sm" />}
      {status.isError && (
        <Alert color="red" variant="light">
          Failed to load Codex status: {(status.error as Error)?.message}
        </Alert>
      )}
      {status.data && (
        <Paper withBorder radius="md" p="md" data-testid="codex-card">
          <Stack gap="sm">
            <CodexStatusBody status={status.data} />
            {status.data.enabled && (
              <Group justify="flex-end">
                <Button
                  size="xs"
                  variant="default"
                  onClick={handleTest}
                  loading={test.isPending}
                  data-testid="codex-test-button"
                >
                  Test connection
                </Button>
                <Button
                  size="xs"
                  variant="light"
                  onClick={handleRefresh}
                  loading={refresh.isPending}
                  data-testid="codex-refresh-button"
                >
                  Refresh now
                </Button>
              </Group>
            )}
          </Stack>
        </Paper>
      )}
    </Stack>
  );
}

function CodexStatusBody({ status }: { status: CodexStatusDto }) {
  if (!status.enabled) {
    return (
      <Alert color="gray" variant="light" data-testid="codex-status-disabled">
        The Codex integration is disabled. Set{" "}
        <code>[codex] enabled = true</code> (with <code>base_url</code> and{" "}
        <code>api_key</code>) to enable it.
      </Alert>
    );
  }
  const auth = AUTH_STATE_META[status.authState] ?? AUTH_STATE_META.unknown;
  return (
    <Stack gap={6} data-testid="codex-status-body">
      <Group gap="xs" wrap="wrap">
        <Badge
          color={status.reachable ? "green" : "red"}
          variant="light"
          data-testid="codex-reachable-badge"
        >
          {status.reachable ? "Reachable" : "Unreachable"}
        </Badge>
        <Badge
          color={auth.color}
          variant="light"
          data-testid="codex-auth-badge"
        >
          {auth.label}
        </Badge>
        {status.codexName && (
          <Text size="sm" c="dimmed">
            {status.codexName}
            {status.codexVersion ? ` v${status.codexVersion}` : ""}
          </Text>
        )}
      </Group>
      {codexVersionOutdated(status.codexVersion) && (
        <Alert
          color="yellow"
          variant="light"
          data-testid="codex-version-warning"
        >
          Codex v{status.codexVersion} is older than v{MIN_CODEX_VERSION}, which
          adds the series index the presence sync needs. Upgrade Codex if syncs
          keep failing.
        </Alert>
      )}
      <Group gap="lg" wrap="wrap">
        <Text size="sm">
          <Text span fw={600}>
            Series:
          </Text>{" "}
          {typeof status.linkedCount === "number" ? status.linkedCount : "—"}
          {" linked"}
          {typeof status.fetchedCount === "number"
            ? ` of ${status.fetchedCount} on Codex`
            : ""}
        </Text>
        <Text size="sm">
          <Text span fw={600}>
            Last sync:
          </Text>{" "}
          {typeof status.lastSuccessAt === "number"
            ? formatRelative(status.lastSuccessAt)
            : "never"}
        </Text>
      </Group>
      {status.lastError && (
        <Text size="xs" c="red" data-testid="codex-last-error">
          Last error: {status.lastError}
        </Text>
      )}
      <CodexRecentChecks checks={status.recentChecks} />
    </Stack>
  );
}

/// Reachability transition history (newest first). Populated by the launch
/// probe, the sync cron, and the manual Test button; empty until something
/// flips.
function CodexRecentChecks({ checks }: { checks: HealthCheckDto[] }) {
  if (checks.length === 0) return null;
  return (
    <Stack gap={4} data-testid="codex-recent-checks">
      <Text size="xs" fw={600} c="dimmed" tt="uppercase">
        Recent checks
      </Text>
      {checks.map((c) => (
        <Group
          key={`${c.checkedAt}-${c.trigger}`}
          gap="xs"
          wrap="wrap"
          align="baseline"
        >
          <Badge
            size="xs"
            variant="light"
            color={c.reachable ? "green" : "red"}
          >
            {c.reachable ? "up" : "down"}
          </Badge>
          <Text size="xs" c="dimmed">
            {c.trigger}
          </Text>
          <Text size="xs" c="dimmed" title={formatAbsolute(c.checkedAt)}>
            {formatRelative(c.checkedAt)}
          </Text>
          {c.error && (
            <Text size="xs" c="red" lineClamp={1} title={c.error}>
              {c.error}
            </Text>
          )}
        </Group>
      ))}
    </Stack>
  );
}
