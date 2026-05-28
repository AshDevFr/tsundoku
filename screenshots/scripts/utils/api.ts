import type { APIRequestContext } from "playwright";
import { config } from "../../playwright.config.js";
import { bearer } from "./auth.js";

export interface StatsResponse {
  series: number;
  releases: {
    resolved: number;
    unresolved: number;
    ambiguous: number;
    reviewPending: number;
    rejected: number;
  };
  totalReleases: number;
  activeProvider: string;
}

interface SourceListItem {
  name: string;
  kind: string;
  enabled: boolean;
}

export async function getStats(
  request: APIRequestContext,
): Promise<StatsResponse | null> {
  const res = await request.get("/api/v1/stats");
  if (!res.ok()) {
    return null;
  }
  return (await res.json()) as StatsResponse;
}

export async function listSources(
  request: APIRequestContext,
): Promise<SourceListItem[]> {
  const res = await request.get("/api/v1/sources");
  if (!res.ok()) {
    return [];
  }
  const body = (await res.json()) as { items?: SourceListItem[] };
  return body.items ?? [];
}

/// Trigger one or more source polls and wait for the resulting releases
/// to land in the DB. Returns true if anything was resolved during the
/// wait window.
export async function pollAndWait(
  request: APIRequestContext,
  sources: string[],
): Promise<boolean> {
  console.log(`  ⏳ Triggering polls for: ${sources.join(", ") || "<none>"}`);

  for (const name of sources) {
    const res = await request.post(`/api/v1/sources/${name}/poll`, {
      headers: bearer(),
    });
    if (!res.ok()) {
      console.log(`    ⚠️  Poll trigger for "${name}" failed: ${res.status()}`);
      continue;
    }
    const body = await res.json().catch(() => ({}));
    console.log(`    → ${name}: ${JSON.stringify(body)}`);
  }

  return waitForReleases(request, config.pollWaitMaxSeconds);
}

/// Poll /stats until totalReleases > 0 or the deadline elapses.
async function waitForReleases(
  request: APIRequestContext,
  maxSeconds: number,
): Promise<boolean> {
  const deadline = Date.now() + maxSeconds * 1000;
  let lastTotal = -1;

  while (Date.now() < deadline) {
    const stats = await getStats(request);
    if (stats && stats.totalReleases > 0) {
      if (stats.totalReleases !== lastTotal) {
        console.log(
          `    … ${stats.totalReleases} releases ` +
            `(resolved=${stats.releases.resolved}, ` +
            `unresolved=${stats.releases.unresolved}, ` +
            `ambiguous=${stats.releases.ambiguous})`,
        );
        lastTotal = stats.totalReleases;
      }
      // Give the resolver another beat to finish — but bail early
      // once resolved + unresolved + ambiguous equals total (i.e. the
      // pipeline has drained).
      const counted =
        stats.releases.resolved +
        stats.releases.unresolved +
        stats.releases.ambiguous +
        stats.releases.reviewPending +
        stats.releases.rejected;
      if (counted >= stats.totalReleases) {
        return true;
      }
    }
    await new Promise((r) => setTimeout(r, 3000));
  }

  console.log("    ⚠️  Poll wait timeout reached, continuing with whatever landed");
  return false;
}
