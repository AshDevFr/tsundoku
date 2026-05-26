---
slug: /
title: Introduction
sidebar_position: 0
---

# tsundoku

**tsundoku** (積ん読 — "pile of unread books") is a manga-discovery
sidecar for [Codex](https://codex.4sh.dev). It polls release sources
like Nyaa, resolves each release to a canonical
[MangaBaka](https://mangabaka.dev) series, and surfaces what you
**don't** own yet.

It deliberately does **not** track releases for series you already
have — Codex's `release-nyaa` plugin does that. tsundoku is the
"discover new things" half of the workflow.

## How it fits together

```
┌────────────────────────────────────────────────────────────┐
│  Discovery sources (Nyaa, ...)                             │
│       │                                                    │
│       ▼                                                    │
│  Resolution pipeline                                       │
│   1. Known external ID (already in the catalog)            │
│   2. Foreign-ID lookup via MangaBaka                       │
│   3. Fuzzy title search (cleaned query, Dice rescore)      │
│   4. Format-to-kind validation                             │
│       │                                                    │
│       ▼                                                    │
│  resolved / ambiguous / review_pending / unresolved        │
└────────────────────────────────────────────────────────────┘
```

Confident matches land in your catalog. Ambiguous ones queue for
human review with cleaned search queries, candidate covers, and a
one-click provider-search modal.

## Where to next

- New here? Start with [Getting Started](./getting-started.mdx).
- Setting up a fresh deploy? [Configuration](./configuration.md) is the
  field-by-field reference.
- Running it? [Sources](./sources.md), [Providers](./providers.md), and
  [Review queue](./review-queue.md) cover day-to-day operations.
- Going to prod? [Deployment](./deployment.md) and
  [Troubleshooting](./troubleshooting.md).
- Curious about the design? [Architecture](./architecture.md) is the
  short tour.

The source lives at
[github.com/skewb1k/tsundoku](https://github.com/skewb1k/tsundoku).
