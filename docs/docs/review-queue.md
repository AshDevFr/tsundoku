---
title: Review queue
sidebar_position: 5
---

# Review Queue

Most releases auto-resolve through the four-step resolution pipeline:
known external ID → foreign-ID lookup → fuzzy title → format-to-kind
validation. The rest land in the **review queue** at `/review`, where
an operator decides what to do with them.

## What lands here

| Status | Why it's here |
|---|---|
| `review_pending` | Fuzzy match scored above `review_threshold` (0.55 default) but below `resolution_threshold` (0.85). Candidates persisted. |
| `unresolved` | No candidate cleared `review_threshold`. No candidates persisted. |
| `ambiguous` | A confident match was found, but its format (e.g. `cbz`) doesn't match the matched series's kind (e.g. `novel`). Linked, but flagged. |

## Card anatomy

Each card shows:

- **Raw title** from the source — what the uploader actually posted.
- **`searched: "..."`** — the cleaned primary search query the
  resolver used. Alternate queries (from `|` or ` / ` separators in
  the raw title) appear as chips.
- **Cleanup-rule badges** — every rule that fired during title
  cleaning: `strip_brackets`, `strip_parens`, `strip_vol_compact`,
  `strip_format`, `split_alternates`, etc. Tells you at a glance what
  surgery happened.
- **Candidate strip** — cover, canonical title, Dice score for each
  scored candidate. One-click "Link" buttons.
- **Actions** — `Search provider`, `Retry`, `Reject`.

If a release auto-resolved via foreign-ID lookup, the search query and
rule badges still appear — they document what the cleaner *would have*
produced, which is useful diagnostic data for the operator and for the
next person debugging a regression.

## Actions

### Link a candidate

Click "Link" on any candidate card. The release transitions to
`resolved` with `resolution_path = "manual"` and the series row is
linked. The release leaves the queue.

### Search provider

Opens the provider search modal:

- **Title input** — pre-filled with the release's cleaned primary
  query. Debounced 300 ms; results show as you type.
- **External ID input** — paste a provider ID for a direct lookup.
  When set, takes priority over title — the modal short-circuits to
  `MetadataProvider::get` and returns a single hit at score 1.0.
- **Result list** — each hit shows cover, title, native title (if
  available), Dice-rescored score, and badges for year / kind /
  status. Click "Link" to confirm.

The modal can search any registered provider, not just the active
one — useful when a series isn't in the active provider's cache but
exists in a sibling provider.

### Retry

Re-runs the full resolution pipeline against the release. Useful
after:

- Refreshing the MangaBaka offline cache (`Refresh cache` on the
  Providers card or `tsundoku refresh-metadata`).
- Tweaking `[ingestion.cleanup.extra_format_keywords]` to handle a
  new uploader keyword.
- Tweaking `resolution_threshold` or `review_threshold`.
- Adding a new format-type rule.

### Reject

Marks the release as `rejected` and drops it from the queue. Use for
spam, off-topic content, or releases that genuinely don't have a
MangaBaka counterpart.

## Bulk operations

Not yet — the queue is one-card-at-a-time. If a release pattern
floods the queue repeatedly (e.g. a new uploader pasting a keyword the
cleaner doesn't know), the fix is usually:

1. Add the keyword to `[ingestion.cleanup.extra_format_keywords]`.
2. Retry the affected releases.

## Auth

Review-queue endpoints are **write** endpoints — they always require
`auth.admin_token` as a `Bearer` token regardless of
`read_requires_auth`. The admin UI prompts for the token on first
load and caches it in `localStorage`. A "Sign out" button on the
review page clears the cached token.

If `admin_token` is unset in config, the server returns `503
Misconfigured` (not `401`) so a fresh deploy without the token
configured doesn't look like a credentialing bug.

## Metrics

The admin metrics tab shows queue dynamics:

- **Depth over time** — `review_queue_snapshots` table, populated at
  minute 5 of every hour by the scheduler.
- **Oldest pending** — age of the oldest still-pending release.
- **Median time to decision** — between landing in the queue and
  getting linked or rejected.

A flat-and-growing depth chart usually means a cleaner rule is missing
or a new uploader pattern needs handling. A spiky chart that drains
between spikes is healthy.
