---
title: Admin maintenance
sidebar_position: 8
---

# Admin maintenance

[`/admin/maintenance`](http://127.0.0.1:8080/admin/maintenance) hosts
cross-provider operational actions that don't fit on a per-source or
per-provider surface. Each entry is a card with its own confirmation
modal; the page is designed to grow siblings (rebuild FTS, clear the
negative cache, etc.) without restructuring.

Every action on this page requires the admin bearer token. Most are
rare; reach for them when a config change or schema migration has left
the persisted catalog out of sync with what the provider currently
publishes.

## Invalidate metadata hashes

Clears the `metadata_hash` column on every provider-backed series row.
The next [series-row refresh](./providers.md#series-row-refresh) tick
rewrites each affected row from the canonical provider metadata
instead of short-circuiting on a hash match. Manual rows are skipped.

Use it when:

- A new denormalized column was added to the `series` table and
  existing rows still show the old shape (e.g. `NULL` volumes or
  chapters after a migration that added `total_volumes`).
- You suspect the persisted metadata has drifted from what the
  provider currently publishes and want to force a full rewrite.

The clear itself is cheap (one `UPDATE`). The work that matters is the
refresh that follows. Either hit **Refresh all series metadata** below
or wait for the next `metadata.series_refresh.cron` tick.

Endpoint: `POST /api/v1/series/invalidate-metadata-hashes` →
`{ invalidated, skippedManual }`.

## Refresh all series metadata

Triggers a series-metadata refresh tick against the active provider
right now, instead of waiting for the next
`metadata.series_refresh.cron` tick. Honors the configured
`batch_size` and `min_age_days`; manual rows are always skipped.

Pair it with **Invalidate metadata hashes** above when adding a
denormalized column. Use it standalone to amortize a backlog after a
fresh dump refresh.

The trigger shares the provider-scoped job lock with the scheduled
cron — a manual trigger that lands while a refresh is in flight
returns `{ triggered: false }` rather than starting a second batch.
The card reflects the result in a toast and the in-flight pill on the
[Providers](./providers.md) page shows live progress while the tick
runs.

Endpoint: `POST /api/v1/series/refresh-all` →
`{ triggered, skipped, batchSize, minAgeDays, provider }`.

## Invalidate cover cache

Deletes every file under the cover-proxy cache directory
(`storage.cover_cache_dir`, default `${data_dir}/cache/covers/`).
Covers are re-fetched on demand the next time the UI requests them,
at the cost of one upstream fetch per series.

Use it when:

- An upstream cover was corrected and the proxy is still serving the
  old bytes (the content-addressed cache keys mean rotated URLs
  refresh automatically, but a corrected file at the **same** URL
  doesn't).
- You want to reclaim disk space.

In-flight browser tabs may continue to render cached bytes until their
own `Cache-Control: max-age=3600` window expires. A hard refresh in
the affected tab picks up the new covers immediately.

Endpoint: `POST /api/v1/covers/invalidate-cache` →
`{ filesDeleted, bytesFreed }`.

See [Architecture → Cover image proxy](./architecture.md#cover-image-proxy)
for the cache layout and the `by-url` host allowlist.
