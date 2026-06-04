---
title: Codex integration
sidebar_position: 10
---

# Codex integration

tsundoku is discovery-centric: it surfaces series you might want, with no
inherent idea of what you already own. The optional **Codex integration**
closes that gap. It periodically reads your [Codex](https://codex.4sh.dev)
library and overlays, on each series, whether you already own it and whether
newer volumes or chapters have surfaced since.

The overlay is **admin-only**. A public (read-tier) visitor never sees it, in
the payload or the UI — what's in your library stays private.

[Codex](https://github.com/AshDevFr/codex) is the self-hosted library server
that holds what you already own; tsundoku reads its series index and projects
that ownership onto your discovery feed.

![Codex home dashboard](/img/manual-screenshots/codex/home-dashboard.png)

## Enabling it

Add a `[codex]` block to your config (see [Configuration](./configuration.md#codex)):

```toml
[codex]
enabled   = true
base_url  = "https://codex.example.com"
api_key   = "codex-reader-api-key"
sync_cron = "0 */6 * * *"   # every 6 hours; omit to disable the cron
```

Two things to get right:

- **`api_key` is Codex's key, not tsundoku's.** It's an `X-API-Key` for a Codex
  account with the `series:read` scope — unrelated to tsundoku's own
  `[auth] api_key` / `admin_token`. You can supply it via the
  `TSUNDOKU_CODEX__API_KEY` environment variable instead of the file.
- **`base_url` does double duty.** It's both the API base for the sync and the
  host used to build the deep links the badges point at
  (`<base_url>/series/{uuid}`). There is no separate deep-link setting.

When `enabled = true`, `base_url` and `api_key` are **required** — tsundoku
refuses to start otherwise, rather than silently failing every sync tick.

## How the sync works

A scheduled job (and a manual trigger) does one sweep:

1. **Preflight.** Pings Codex's public `GET /api/v1/info` to confirm it's
   reachable and log its name/version. If Codex is down, the sweep is skipped
   and your existing ownership data is left untouched — a transient outage
   never wipes the overlay.
2. **Sweep.** Pulls Codex's series index and matches each entry to a tsundoku
   series by shared external ID (MangaBaka / AniList / MangaUpdates / MAL).
3. **Persist.** Records the link plus Codex's owned volume/chapter maxima
   locally, so browsing only joins local data — no per-request calls to Codex.

A successful `/info` proves Codex is reachable; it does **not** prove your
`api_key` is valid. The first authenticated sweep is what validates the key —
see [Connection status](#connection-status) below.

## What the badges mean

On the feed (cards and list rows) and the series detail page, an owned series
gets a colored badge:

| Badge | Meaning |
| --- | --- |
| 🟢 **owned** | On Codex and caught up — Codex owns at least as much as has surfaced. |
| 🔵 **behind** | On Codex, but newer volumes/chapters have surfaced than Codex owns. Worth grabbing. |
| ⚪ **owned?** | On Codex, but the relevant count didn't parse on Codex's side, so we can't tell if you're caught up. |
| *(no badge)* | Not on Codex (or you're not signed in as admin). |

The status compares the **highest volume/chapter number** discovered from
releases against the **highest number Codex owns**. It deliberately ignores
Codex's raw owned-file count (shown only as an approximate "~N owned" tooltip):
that count can't tell volumes from chapters reliably, so it's display-only and
never drives the badge color.

Click a badge to open the series in Codex.

> **Note:** a series surfaced as volumes but owned on Codex only as loose
> chapters (or vice-versa) can read as **behind** even when you effectively
> have it. Blue means "look at this," which is the safe direction to err.

## Filtering by ownership

The feed's **Codex** filter (admin-only) narrows the list by status: on Codex
(any), up to date, behind, unverified, or not on Codex. The filter is enforced
server-side — a non-admin request carrying the filter just gets the normal
unfiltered feed, so it can't be used to probe your library.

## Connection status

The admin **Codex** page (`/admin/codex`) shows the live connection health:

![Codex integration page](/img/screenshots/admin/codex.png)

- **Reachable / Unreachable** — whether the last preflight reached Codex.
- **Auth state** — `Connected`, or one of two distinct failures:
  - **API key rejected (401)** — the key is wrong or missing. Fix `codex.api_key`.
  - **API key lacks series:read (403)** — the key authenticates but the Codex
    account doesn't have the `series:read` scope. Grant it on the Codex side.
- **Linked series**, **last sync**, and the **last error** message.
- A **Test connection** button to probe Codex on demand, and a **Refresh now**
  button to trigger a sweep immediately instead of waiting for the cron.
- A **Recent checks** reachability history (up/down transitions) alongside a
  **Recent syncs** sweep history (per-sweep outcome, linked/fetched counts, or
  the error that failed it).

Badges stay hidden until the first successful sweep, so a freshly-enabled
integration doesn't briefly show everything as "not owned."

## Series Codex can't match automatically

A series with no external ID tsundoku shares with Codex (a manual entry, or one
whose IDs didn't line up) won't auto-link. The API exposes
`POST /api/v1/series/{id}/codex-link` (and `DELETE` to unlink) to hand-link such
a series to a Codex UUID; the next sweep then fills in its owned counts. A UI
for this is not yet wired up — use the API directly for now.
