---
title: Feature showcase
sidebar_position: 1
---

# Feature showcase

A guided tour through the things tsundoku does, with the screens
captured live against a running install. Pages link out to the
operator guides where each feature is described in detail.

## Browse the catalog

The feed at `/` is the surface most of the rest of the docs link
back to. Every series tsundoku has linked at least one release to
shows up here, filterable and ordered however you like.

### Card grid

The default view, sized for cover-driven browsing. Each card shows
the canonical title, kind, status, year, and how recently the last
release landed.

![Browse feed in card view](/img/screenshots/browse/feed-cards.png)

### List view

Denser layout for sort-by-count comparisons (total volumes, total
chapters, release counts). The view toggle is in the page header and
is sticky via the `view=list` URL param.

![Browse feed in list view](/img/screenshots/browse/feed-list.png)

### Filter rail

The left rail lives in the URL — copy the address bar to share or
bookmark a view. Genre and tag chips can be combined with **Any**
(default) or **All** semantics. Releases is a tri-state for the
orphan / has-releases axis.

![Browse feed with the filter panel](/img/screenshots/browse/feed-with-filters.png)

→ Full rundown of every filter, preset, and sort: [Browse](./browse.md).

## Series detail

Click any card to drill into the full series page. Description,
alternate titles, every external-ID mapping the resolver has on
file, and the full release list — including a per-row **Move**
action to re-link a release that landed on the wrong series.

![Series detail page](/img/screenshots/series/detail.png)

## Admin

Everything operator-facing lives under `/admin/*` and is gated by
`auth.admin_token`. The shell prompts for the token on first load
and caches it in localStorage.

### Overview

Health-at-a-glance for every registered source and provider, plus
queue counters. A red **N failing** pip in the page header surfaces
sources whose last poll errored.

![Admin overview](/img/screenshots/admin/overview.png)

### Review queue

Releases that the resolver wasn't sure about. Each card surfaces
the raw title, the cleaned search query, every cleanup rule that
fired, scored candidates with one-click **Link**, and the
provider-search modal for the cases where none of the candidates
fit.

![Review queue](/img/screenshots/admin/review.png)

→ Card anatomy, bulk operations, and the re-resolve-after-rules-change
flow: [Review queue](./review-queue.md).

### Kept releases

The resting place for standalone items — guidebooks, artbooks,
databooks — that aren't a series but also shouldn't be rejected.
Skipped by the resolver; only the manual **Re-resolve** button on
each card sends them back through the pipeline.

![Kept releases](/img/screenshots/admin/kept.png)

→ When to use Keep vs Reject: [Kept releases](./kept-releases.md).

### Sources

Every `[[sources]]` block from config shows up here. Last poll
time, last summary (new vs duplicate), last error, and a per-source
**Poll now** button that shares its job lock with the cron tick.

![Sources list](/img/screenshots/admin/sources-list.png)

The per-source detail page shows recent runs, the full effective
config, and historical metrics.

![Source detail](/img/screenshots/admin/source-detail.png)

→ Adding a source, RSS feed URL recipes, and the backfill caveat:
[Sources](./sources.md).

### Providers

The metadata-provider equivalent: every registered
`[providers.<id>]` block, with the active one marked. Surfaces
offline-cache age, last refresh status, and a **Refresh cache**
trigger that downloads the latest dump.

![Providers list](/img/screenshots/admin/providers-list.png)

Per-provider detail surfaces the offline dump's row count, the
SHA-1 sidecar that gates re-extraction, and a recent-runs table.

![Provider detail](/img/screenshots/admin/provider-detail.png)

→ Active vs registered, the offline-dump lifecycle, and the API
fallback toggle: [Providers](./providers.md).

### Metrics

Time-series charts for source poll volume, resolver throughput,
review-queue depth, and per-provider call counts. The depth chart
is fed by the hourly `review_queue_snapshots` snapshot job, so a
flat-and-growing line means a cleaner rule is missing.

![Metrics dashboard](/img/screenshots/admin/metrics.png)

### ID maps

Cross-provider foreign-ID lookups (MangaUpdates ⇄ MangaBaka,
etc.). Used by the resolver's foreign-ID step to short-circuit
fuzzy title search when an external link extracted from a post
body matches a known canonical id.

![ID maps page](/img/screenshots/admin/id-maps.png)

### Maintenance

Escape-hatch operations that don't belong on a per-source or
per-provider page: invalidate metadata hashes so the next refresh
rewrites every row, kick a series-metadata refresh tick now instead
of waiting for cron, or wipe the cover-proxy disk cache. Each is a
card with its own confirmation modal.

![Maintenance page](/img/screenshots/admin/maintenance.png)

→ When to reach for each card: [Admin maintenance](./admin-maintenance.md).
