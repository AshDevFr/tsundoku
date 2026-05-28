---
title: Providers
sidebar_position: 4
---

# Metadata Providers

A **provider** turns a series ID into canonical metadata. tsundoku's
resolution pipeline calls the active provider's `resolve_by_foreign_id`
and `search` methods to match incoming releases to series.

v1 ships only **MangaBaka**, but the architecture is provider-pluggable
via the
[`MetadataProvider`](https://github.com/skewb1k/tsundoku/blob/main/crates/td-metadata/src/provider.rs)
trait. Adding one means writing a `td-metadata-<name>` crate plus a
config block — no core changes.

## Active vs. registered

Multiple providers can be **registered** (so the review UI can search
across them and the resolver can chain foreign-ID lookups), but exactly
one is designated `metadata.active_provider` and runs the
auto-resolution path. Switching is a config-level decision:

```toml
[metadata]
active_provider = "mangabaka"
```

## MangaBaka

MangaBaka is the v1 active provider. The provider's design is
**offline-first**: a nightly SQLite dump is downloaded, opened
read-only as a side database, and queried via the bundled FTS5 mirror.
Live API calls only fire when explicitly enabled via `api_fallback`.

### Offline dump lifecycle

MangaBaka publishes nightly dumps at
`https://api.mangabaka.dev/v1/database/series.sqlite.tar.gz`
(~476 MB compressed).

`tsundoku refresh-provider-cache`:

1. Downloads the tarball + the SHA-1 sidecar.
2. Verifies the hash.
3. Extracts to `${data_dir}/cache/providers/mangabaka/series.sqlite`.
4. Adds 8 source-id indexes + an FTS5 mirror for fast title search.

The extracted dump is opened read-only as a side database — queries
run against MangaBaka's canonical rows directly. We don't re-ingest
the 585k rows into provider-owned tables; drift would mean we're
wrong, and the file is the source of truth.

### Scheduled refresh

```toml
[providers.mangabaka]
offline_refresh_cron = "0 4 * * 0"   # weekly Sunday 04:00 UTC
```

The scheduler will hit MangaBaka's dump endpoint at that cadence and
swap the on-disk cache atomically when verification succeeds. A
failure leaves the previous dump in place — the resolver keeps
working against the stale version until the next successful refresh.

5-field crons are auto-padded to seconds-0.

### `api_fallback`

```toml
[providers.mangabaka]
api_key       = "mb-..."
api_fallback  = true
```

When `api_fallback = true` **and** `api_key` is set, cache misses
trigger a live API call against `api_base_url` to fetch the canonical
record. Successful calls populate the in-memory cache so a repeat
within the same process is free; the next dump refresh picks up the
mapping permanently.

Without an `api_key`, the provider runs offline-only. Cache misses
surface as `Ok(None)` to the resolver and the release falls through
to fuzzy-title search.

### Negative cache

`negative_cache_ttl_days = 7` controls how long the provider remembers
"this foreign ID isn't in MangaBaka." Tombstone entries prevent
repeated API hits for IDs that genuinely don't exist. Adjust based on
how often you expect MangaBaka to backfill new entries.

### Manual refresh

From the admin UI: **Refresh cache** on the MangaBaka card.

From the CLI:

```bash
tsundoku refresh-provider-cache                       # all providers
tsundoku refresh-provider-cache --provider mangabaka  # one specific provider
```

Both paths share the same `JobLocks` mutex with the cron job — manual
and scheduled refreshes can't race.

## Series-row refresh

`refresh-provider-cache` swaps the provider's dump. It does **not**
touch any `series` row — the canonical title, description, cover URL,
genres, tags, volume / chapter counts, and rating stored on each
catalog row keep whatever values the resolver wrote when it first
matched the release.

Over time those rows drift from the provider's current state
(MangaBaka backfills, the series advances, the description gets
edited). The series-row refresh job walks the catalog and re-fetches
each row's metadata from the active provider, persisting the result.

### Config

```toml
[metadata.series_refresh]
cron          = "0 5 * * *"   # daily 05:00 UTC. Omit to disable the cron.
batch_size    = 50            # max rows refreshed per tick. 0 = no-op (transient disable).
min_age_days  = 7             # skip rows whose metadata is fresher than this.
```

`min_age_days` defaults to 7 to match MangaBaka's published-dump
cadence — refreshing a row whose metadata is one day old is wasted
work because the cache hasn't moved. Loosen or tighten based on the
upstream provider's churn.

5-field crons are auto-padded to seconds-0, like every other cron in
tsundoku.

### Triggers

| Path | Use |
|---|---|
| Scheduled cron (`metadata.series_refresh.cron`) | Hands-off; runs continuously while `serve` is up. |
| `POST /api/v1/series/refresh-all` | Kick a batch now, from the admin UI or `curl`. Returns `202` with `{ triggered, skipped, batchSize, minAgeDays, provider }`. |
| `tsundoku refresh-series` | Same code path; for cron-from-outside or one-shot batches when `serve` isn't running. Accepts `--batch-size` / `--min-age-days` overrides. |
| `POST /api/v1/series/{id}/refresh-metadata` | Refresh a single series row by id — bypasses `min_age_days`, runs synchronously. Used by the admin UI's per-series action. |

All bulk paths share the same provider-scoped job lock as
`refresh-provider-cache`: a manual trigger that lands while a refresh
is already in flight returns `triggered: false, skipped: true` rather
than starting a second batch.

A series with no mapping for the active provider is skipped silently
on the bulk path; the single-id endpoint returns `409 Conflict` so
the operator knows the row needs a manual link first.

## Metrics

Every refresh attempt writes a row to `provider_refreshes`. Surfaced
on the admin metrics tab:

- Bytes downloaded per refresh.
- Record count after extraction.
- P50 / P95 dump-download latency (just the HTTP portion, separate
  from extract + indexing time).
- Error-kind donut on failures, same enum as `poll_runs`.

## Adding a new provider

Implement the
[`MetadataProvider`](https://github.com/skewb1k/tsundoku/blob/main/crates/td-metadata/src/provider.rs)
trait in a new `td-metadata-<name>` crate, add a `[providers.<name>]`
config block, and add one line to the registry builder. If the
provider has an offline dump, ship a nested sea-orm migrator inside
the provider crate; the top-level `migration::Migrator` composes it.
No core changes.

## Configuration reference

See [Configuration → providers.mangabaka](./configuration.md#providersmangabaka).
