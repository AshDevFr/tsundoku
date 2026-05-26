---
title: Architecture
sidebar_position: 7
---

# Architecture

A non-contributor overview of how tsundoku is put together. For the
contributor-level deep dive, read `CLAUDE.md` in the repo.

## One process, single SQLite file

tsundoku is a single Rust binary built on axum 0.8, tokio, and
sea-orm. The HTTP server, the cron scheduler, the resolver, and the
metadata-provider implementations all run in one process.

State lives in **one SQLite file** at `${data_dir}/db/tsundoku.db`.
The pool is pinned to a single connection so per-connection PRAGMAs
(`foreign_keys = ON`, `busy_timeout = 5000`) actually stick. No
multi-writer model, no read replicas, no PostgreSQL fallback.

## Three layers

```
┌──────────────────────────────────────────────────────────┐
│  HTTP API (axum, td-api)                                 │
│  + admin SPA (React 19 + Mantine 9, embedded via         │
│  rust-embed behind the embed-frontend feature)           │
└──────────────────────────────────────────────────────────┘
                          │
┌──────────────────────────────────────────────────────────┐
│  Resolution pipeline (td-resolution)                     │
│                                                          │
│  1. Known external ID  (catalog hit)                     │
│  2. Foreign-ID lookup  (active provider)                 │
│  3. Fuzzy title        (cleaned query, Dice rescore)     │
│  4. Format → kind validation                             │
└──────────────────────────────────────────────────────────┘
        │                                  │
┌───────┴────────────────┐    ┌────────────┴───────────────┐
│ Discovery sources      │    │ Metadata providers         │
│ (td-source +           │    │ (td-metadata +             │
│  td-source-nyaa)       │    │  td-metadata-mangabaka)    │
└────────────────────────┘    └────────────────────────────┘
```

### Sources

A **source** polls some upstream feed and emits `DiscoveredRelease`
records. v1 ships only Nyaa (RSS feed + per-post HTML enrichment).
The [`DiscoverySource`](https://github.com/skewb1k/tsundoku/blob/main/crates/td-source/src/source.rs)
trait is the contract; adding a new source means writing a
`td-source-<name>` crate.

### Resolution pipeline

The pipeline turns a raw release row into either a `series_id` link
(resolved) or a review-queue card. It runs four steps in a fixed
order:

1. **Known external ID** — short-circuit if the release's external
   links already point at a series the catalog knows about.
2. **Foreign-ID lookup** — ask the active provider whether it
   recognizes the foreign IDs (MangaUpdates, AniList, MAL, MangaDex).
3. **Fuzzy title** — clean the raw title (strip parens, brackets,
   volume markers, format keywords, year tokens, split on multi-title
   separators), search the active provider, Dice-rescore against the
   cleaned query, keep the best hit.
4. **Format-to-kind validation** — once a candidate is chosen, check
   that the release's detected formats are consistent with the
   series's kind. A mismatch demotes the release to `ambiguous`.

Confident matches auto-resolve. Plausible-but-low-confidence matches
land in the [review queue](./review-queue.md).

### Providers

A **provider** is a metadata source the resolver talks to. v1 ships
MangaBaka with an offline-first design — a nightly SQLite dump opened
read-only as a side database, queried via an FTS5 mirror. The
[`MetadataProvider`](https://github.com/skewb1k/tsundoku/blob/main/crates/td-metadata/src/provider.rs)
trait is the contract; adding a new provider means writing a
`td-metadata-<name>` crate.

Multiple providers can be registered, but exactly one is designated
`metadata.active_provider` and runs the auto-resolution path.

## Scheduler

`tokio-cron-scheduler` on top of a `JobLocks` map of per-source and
per-provider `tokio::Mutex` instances. Each cron job `try_lock`s its
key; if another tick (or a manual trigger) is already in flight, the
new tick is dropped with a debug log. Manual `POST /sources/{name}/poll`
shares the same lock — manual and scheduled work can't race.

## Real-time updates

Currently: TanStack Query polling on the frontend. No SSE, no
WebSockets. Discovery is a low-frequency activity (polls run on cron,
minutes apart); the operational cost of a push channel would buy
nothing here. A future phase may revisit if a workflow needs live
push.

## Auth model

Single-user, single-host, single SQLite file. Auth is config-driven:

- `auth.read_requires_auth = false` → reads are public.
- `auth.read_requires_auth = true` + `auth.api_key = "..."` → reads
  require the key.
- Writes always require `auth.admin_token` as a `Bearer` token.
- Missing `admin_token` returns `503 Misconfigured` (distinct from
  `401`) so fresh deploys don't look like credentialing bugs.

No users table, no sessions, no JWT. If multi-user becomes a
requirement, that's a major rewrite — not a flag flip.

## Why standalone instead of a Codex plugin?

Codex's release-tracking flow is matched-by-default
(alias-driven). tsundoku is **unmatched-by-default**: it scans
firehoses for series the user has not yet imported. Bolting that
shape onto Codex would permanently bloat its schema for a workflow
that doesn't generalize. The `series.owned` column on `series` is
reserved as a future hook; how it gets populated depends on what
Codex's HTTP API exposes when that integration happens.

## Why SQLite?

Single-user, single-author, single-host workload. The biggest live
table after a year of polling is well under a million rows. Postgres
would be operational overhead with no payoff at this scale. If the
workload ever crosses multi-writer territory, `sea-orm`'s
`sqlx-postgres` feature is a flag flip — but no current path leads
there.
