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
  // The list endpoint nests the config block; `enabled` lives there
  // rather than at the top level.
  config: { enabled: boolean };
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

/// Fire off the source polls without waiting. Returns immediately so
/// the caller can spend the soak window on other work.
export async function triggerPolls(
  request: APIRequestContext,
  sources: string[],
): Promise<void> {
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
}

/// Block until the soak window elapses. Always waits at least
/// `minSeconds`; if the resolver hasn't drained by then, keeps waiting
/// up to `maxSeconds`. The minimum window gives MangaBaka enrichment +
/// cover fetches time to surface richer screenshots than the resolver
/// drain alone would.
export async function waitForPollSoak(
  request: APIRequestContext,
  minSeconds: number = config.pollWaitMinSeconds,
  maxSeconds: number = config.pollWaitMaxSeconds,
): Promise<void> {
  const start = Date.now();
  const minDeadline = start + minSeconds * 1000;
  const maxDeadline = start + Math.max(minSeconds, maxSeconds) * 1000;
  let lastTotal = -1;
  let lastResolved = -1;

  const tick = async (): Promise<boolean> => {
    const stats = await getStats(request);
    if (!stats) return false;
    if (
      stats.totalReleases !== lastTotal ||
      stats.releases.resolved !== lastResolved
    ) {
      const elapsed = Math.round((Date.now() - start) / 1000);
      console.log(
        `    [+${elapsed}s] ${stats.series} series, ${stats.totalReleases} releases ` +
          `(resolved=${stats.releases.resolved}, ` +
          `unresolved=${stats.releases.unresolved}, ` +
          `ambiguous=${stats.releases.ambiguous}, ` +
          `review=${stats.releases.reviewPending})`,
      );
      lastTotal = stats.totalReleases;
      lastResolved = stats.releases.resolved;
    }
    const counted =
      stats.releases.resolved +
      stats.releases.unresolved +
      stats.releases.ambiguous +
      stats.releases.reviewPending +
      stats.releases.rejected;
    return stats.totalReleases > 0 && counted >= stats.totalReleases;
  };

  // Phase 1: always wait the full minimum, logging progress along the
  // way. Don't early-exit on drain — we want the soak time for the
  // metadata layer.
  while (Date.now() < minDeadline) {
    await tick();
    await new Promise((r) => setTimeout(r, 5000));
  }

  // Phase 2: if drain has happened by now, stop. Otherwise keep going
  // until the hard ceiling.
  while (Date.now() < maxDeadline) {
    if (await tick()) {
      console.log("    ✓ Resolver drained");
      return;
    }
    await new Promise((r) => setTimeout(r, 5000));
  }

  console.log("    ⚠️  Soak ceiling reached, continuing with whatever landed");
}
