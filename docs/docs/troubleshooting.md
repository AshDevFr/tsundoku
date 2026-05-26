---
title: Troubleshooting
sidebar_position: 8
---

# Troubleshooting

Common failure modes and how to diagnose them. If your symptom isn't
here, check the structured logs (`logging.json = true` makes them
greppable) and the admin metrics tab — most operational questions are
answerable from one of those two surfaces.

## Resolver

### Many releases land in the review queue

Check the **review queue cards** themselves. Every card shows the
cleaned search query the resolver used and the cleanup rules that
fired. Common patterns:

- **Cleaner missed a keyword.** Look at uploader-specific patterns
  (e.g. a new keyword like `Remastered`). Add to
  `[ingestion.cleanup.extra_format_keywords]` and use the **Retry**
  button on the queue card.
- **Dice score just under threshold.** The candidate is right but
  noisy title text drags the score. If this happens systematically
  for a known-good release shape, lower `resolution_threshold` (e.g.
  from 0.85 to 0.80). Don't drop it below 0.70 — false-positive risk
  rises sharply.
- **No candidates at all.** The cleaned query produced no FTS hits.
  Try the **Search provider** modal manually — if the series is in
  MangaBaka, the cleaner is being too aggressive; flag it as a
  bug. If MangaBaka doesn't have it, **Reject** the release.

### A release auto-resolved to the wrong series

Use the **Search provider** modal to find the correct series, then
click "Link" on the right hit. The release transitions to `resolved`
with `resolution_path = "manual"`. The wrong link is overwritten.

If a *pattern* of wrong matches surfaces (same uploader, same shape),
file an issue with the cleaned query + the candidate's Dice score —
the cleaner rule order may need a tweak.

### Provider returned `503 Misconfigured`

The `auth.admin_token` config value is unset. Set it and restart. The
`503` is intentional — distinct from `401 Unauthorized` so a fresh
deploy doesn't look like a credentialing bug.

## Sources

### Nyaa source returns zero releases

Check the admin Sources card:

- **`last_error`** — if populated, the feed URL is wrong or Nyaa is
  unreachable. Verify with `curl <feed_url>` from the host.
- **`fetched_count = 0`** but no error — the feed is responding but
  empty for the configured filters. Try a less restrictive feed URL
  in a browser.
- **Stale `last_polled_at`** — the scheduler isn't running the source.
  Verify `cron` is set and the source is `enabled = true`. The
  `tsundoku poll --source <name>` one-shot also works for diagnosis.

### MangaUpdates legacy ID resolution looks stuck

The `mangaupdates_id_map` table caches per-legacy-ID translations,
including tombstones for dead IDs. To inspect:

```bash
sqlite3 ${data_dir}/db/tsundoku.db \
  'SELECT legacy_id, modern_id, resolved_at FROM mangaupdates_id_map ORDER BY resolved_at DESC LIMIT 20'
```

Rate-limit recovery: the resolver throttles at 1 req/sec per host
with exponential backoff on `429`. A long outage tightens the next-
allowed timestamp; failed lookups don't cache, so the next poll
retries naturally.

## Providers

### `Refresh cache` button takes a long time

The MangaBaka dump is ~476 MB compressed. First-time refresh fetches,
verifies SHA-1, extracts, and adds 8 indexes + an FTS5 mirror. On a
modest machine this can take 3–5 minutes. Subsequent refreshes are
faster because rotation reuses the extraction path.

If `refresh-metadata` hangs longer than that, kill it and check the
host's network. The download supports resume via HTTP Range; rerun
the refresh and it picks up where it left off.

### Cache misses despite a recent refresh

Two scenarios:

- The series is genuinely missing from MangaBaka's dump. Enable
  `api_fallback = true` + `api_key = "..."` to fall back to the live
  API for cache misses.
- The series exists but under a different foreign ID. Use the
  **Search provider** modal to find it manually.

A negative-cache tombstone may also be holding back retries. The TTL
is `negative_cache_ttl_days` (default 7).

## Database

### "database is locked" errors

The connection pool is pinned to one writer, so this should be rare.
If you see it: another process is holding a write lock. Verify only
one tsundoku instance is running against the data dir.

The `busy_timeout` is 5000 ms — if the offending writer doesn't
release within that window, you'll see the error.

### Reset to a clean slate

```bash
systemctl stop tsundoku
rm ${data_dir}/db/tsundoku.db
tsundoku migrate
# (optional) tsundoku refresh-metadata
systemctl start tsundoku
```

This drops every release, every series, every review-queue card. The
MangaBaka offline cache survives — that's a separate file at
`${data_dir}/cache/providers/mangabaka/series.sqlite`. Delete it too
if you want a full reset.

## Logging

`tracing-subscriber` honors `RUST_LOG`. Examples:

```bash
RUST_LOG=tsundoku=debug tsundoku serve
RUST_LOG=tsundoku=trace,sea_orm=warn tsundoku serve
RUST_LOG=tsundoku=debug,td_resolution=trace tsundoku serve
```

For JSON-structured logs (suitable for Loki / journald aggregators),
set `logging.json = true` in config.

## When to file an issue

[github.com/skewb1k/tsundoku/issues](https://github.com/skewb1k/tsundoku/issues).

Useful in the report:

- tsundoku version (`tsundoku --version` or the GHCR tag).
- The relevant config block (redact `api_key` and `admin_token`).
- Logs at `debug` level around the failing operation.
- For resolution bugs: the raw title from Nyaa, the cleaned query the
  card shows, and a screenshot of the review card if possible.
