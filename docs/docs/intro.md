---
slug: /
title: Introduction
sidebar_position: 1
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
│   3. Fuzzy title search                                    │
│   4. Format-to-kind validation                             │
│       │                                                    │
│       ▼                                                    │
│  resolved / ambiguous / review_pending / unresolved        │
└────────────────────────────────────────────────────────────┘
```

Confident matches land in your catalog. Ambiguous ones queue for
human review with cleaned search queries, candidate covers, and a
one-click provider-search modal.

## What's next

The rest of the documentation is being migrated from the project's
[README](https://github.com/skewb1k/tsundoku#readme). For now, the
README is the operator's reference; this site will catch up over the
next iteration.

Until then:

- **Quick start, configuration, deployment** — see the
  [README on GitHub](https://github.com/skewb1k/tsundoku#readme).
- **Live API reference** — run the binary locally and visit
  `/docs` for the Scalar UI generated from the OpenAPI spec. A
  static copy will land here as part of the next iteration.
- **Source code, issues, contributions** —
  [github.com/skewb1k/tsundoku](https://github.com/skewb1k/tsundoku).
