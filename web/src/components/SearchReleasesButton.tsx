import { Button, Menu, Tooltip } from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useSearchReleases } from "@/api/mutations";
import { useSearchEntries, useSearchRuns } from "@/api/queries";
import { useAdminAuth } from "@/stores/auth";

/// Server-side release search for one series, driven by the configured
/// `[[search]]` endpoints.
///
/// Strictly admin-only (never rendered for anon sessions, on top of the
/// endpoints being admin-gated server-side) and self-gating: nothing
/// renders when no `[[search]]` entries are configured. The primary click
/// searches the default entry; the caret lists every entry, mirroring the
/// send-to-client split-button shape. The walk runs server-side; this
/// component polls the series' `search_runs` until the launched run
/// completes, then notifies with the new-release count and refreshes the
/// release lists.
export function SearchReleasesButton({ seriesId }: { seriesId: number }) {
  const hasAdmin = useAdminAuth((s) => Boolean(s.token));
  const entries = useSearchEntries();
  const trigger = useSearchReleases();
  const qc = useQueryClient();
  // Baseline run id at launch time: the launched run is the first row
  // with a higher id, so a previously-completed run can't be mistaken
  // for the fresh result. `null` ⇒ not watching.
  const [watchFromId, setWatchFromId] = useState<number | null>(null);
  const runs = useSearchRuns(
    hasAdmin ? seriesId : undefined,
    watchFromId !== null,
  );

  const newest = runs.data?.items?.[0];
  const running = newest?.outcome === "running";

  useEffect(() => {
    if (watchFromId === null || !newest) return;
    if (newest.id <= watchFromId || newest.outcome === "running") return;
    setWatchFromId(null);
    if (newest.outcome === "success") {
      const n = newest.releasesNew ?? 0;
      notifications.show({
        color: n > 0 ? "green" : "gray",
        message:
          n > 0
            ? `Search found ${n} new release${n === 1 ? "" : "s"}`
            : "Search finished: no new releases",
      });
    } else {
      notifications.show({
        color: "red",
        title: "Search failed",
        message: newest.error ?? "unknown error",
      });
    }
    // New releases may have resolved to this series (or any other).
    qc.invalidateQueries({ queryKey: ["series-releases"] });
    qc.invalidateQueries({ queryKey: ["series-detail"] });
    qc.invalidateQueries({ queryKey: ["series-list"] });
    qc.invalidateQueries({ queryKey: ["stats"] });
  }, [watchFromId, newest, qc]);

  if (!hasAdmin) return null;
  const items = entries.data?.items ?? [];
  if (items.length === 0) return null;
  const defaultEntry = items.find((e) => e.default) ?? items[0];

  const launch = (name: string) => {
    trigger.mutate(
      { seriesId, search: name },
      {
        onSuccess: (resp) => {
          if (resp.skipped) {
            notifications.show({
              color: "yellow",
              message: `A "${resp.search}" search is already running; try again shortly`,
            });
            return;
          }
          setWatchFromId(newest?.id ?? 0);
        },
        onError: (e) =>
          notifications.show({
            color: "red",
            title: "Search failed to start",
            message: (e as Error).message,
          }),
      },
    );
  };

  const busy = trigger.isPending || running || watchFromId !== null;

  return (
    <Button.Group>
      <Tooltip
        label={`Search "${defaultEntry.name}" for every title of this series and ingest the results.`}
      >
        <Button
          size="compact-xs"
          variant="subtle"
          color="gray"
          onClick={() => launch(defaultEntry.name)}
          loading={busy}
          data-testid="search-releases"
        >
          ⛁ Search releases
        </Button>
      </Tooltip>
      {items.length > 1 && (
        <Menu position="bottom-end" withinPortal>
          <Menu.Target>
            <Button
              size="compact-xs"
              variant="subtle"
              color="gray"
              px={4}
              aria-label="Search endpoint options"
              disabled={busy}
              data-testid="search-releases-options"
            >
              ▾
            </Button>
          </Menu.Target>
          <Menu.Dropdown>
            <Menu.Label>Search with…</Menu.Label>
            {items.map((e) => (
              <Menu.Item
                key={e.name}
                onClick={() => launch(e.name)}
                data-testid={`search-releases-entry-${e.name}`}
              >
                {e.name}
                {e.default ? " (default)" : ""}
              </Menu.Item>
            ))}
          </Menu.Dropdown>
        </Menu>
      )}
    </Button.Group>
  );
}
