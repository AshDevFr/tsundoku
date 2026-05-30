---
title: Configuration
sidebar_position: 2
---

# Configuration

Config files live in `config/` and are picked up automatically. TOML
and YAML are both supported (the parser is chosen by file extension).
Any value can be overridden by an environment variable using the
`TSUNDOKU_` prefix and `__` for nesting:

```bash
TSUNDOKU_SERVER__PORT=9000 \
TSUNDOKU_STORAGE__DATA_DIR=/var/lib/tsundoku \
  tsundoku serve
```

A sibling `<stem>.local.<ext>` file (e.g. `tsundoku.docker.local.toml`)
gets auto-merged on top of the base file before env vars apply. The
local file is gitignored — use it for secrets and host-specific
overrides.

The canonical reference is
[`config/tsundoku.example.toml`](https://github.com/AshDevFr/tsundoku/blob/main/config/tsundoku.example.toml)
with inline documentation for every key. This page is a faster-to-skim
version organized by section.

## `[server]`

```toml
[server]
host = "127.0.0.1"
port = 8080
```

Bind address for the HTTP server. Behind a reverse proxy you'll
typically set `host = "0.0.0.0"` and let the proxy terminate TLS.

## `[storage]`

Single on-disk root. The database, every provider's offline cache, the
cover cache, and scratch space all live here. Each subpath defaults to
a subdirectory; override individually if you need to.

```toml
[storage]
data_dir = "./data"
# database_path      = "./data/db/tsundoku.db"
# provider_cache_dir = "./data/cache/providers"
# cover_cache_dir    = "./data/cache/covers"
# tmp_dir            = "./data/tmp"
```

| Default                        | Contents                                               |
| ------------------------------ | ------------------------------------------------------ |
| `${data_dir}/db/tsundoku.db`   | SQLite database (the only stateful file)               |
| `${data_dir}/cache/providers/` | Metadata provider offline caches (e.g. MangaBaka dump) |
| `${data_dir}/cache/covers/`    | On-disk cache served by the `/api/v1/covers/*` proxy   |
| `${data_dir}/tmp/`             | Transient downloads, in-progress ingests               |

Docker mounts a single volume at `data_dir`. Back up by copying the
directory.

## `[logging]`

```toml
[logging]
level = "info"
json = false
```

`level` is parsed by `tracing-subscriber`'s
[`EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)
syntax — accepts `info`, `debug`, `tsundoku=trace,sea_orm=warn`, etc.
Set `json = true` when log aggregators want structured output.

## `[api]`

```toml
[api]
docs = true
```

Mount the Scalar API docs UI at `/docs`. Disable in hardened
deployments where you don't want a docs surface exposed.

## `[auth]`

Single-user auth from config, not a users table.

```toml
[auth]
read_requires_auth = false
# api_key      = "read-...........-only"
# admin_token  = "write-..........-only"
```

- Reads are public by default. Flip `read_requires_auth = true` and
  set `api_key`; the frontend sends the key via `X-API-Key` or
  `Authorization: Bearer`.
- Write endpoints (review queue, manual polls, manual cache refresh,
  retry/reject) **always** require `admin_token` as a bearer token.
- A missing `admin_token` returns `503 Misconfigured` (distinct from
  `401 Unauthorized`) so a fresh deploy doesn't look like a
  credentialing bug.

## `[metadata]`

```toml
[metadata]
active_provider = "mangabaka"
```

Exactly one provider runs the auto-resolution path. Others may be
registered (for cross-provider foreign-ID chains and review-UI
search), but only `active_provider` drives the resolver's fuzzy step.
Switching is a config-level decision, not a runtime API call.

### `[metadata.series_refresh]`

```toml
[metadata.series_refresh]
# cron        = "0 5 * * *"  # disabled by default; omit to keep it off
batch_size    = 50
min_age_days  = 7
```

Re-fetches catalog series rows from the active provider so the
stored title, description, cover, genres, tags, counts, and rating
keep up with upstream changes. Distinct from
[`providers.mangabaka.offline_refresh_cron`](#providersmangabaka),
which swaps the provider's dump but never touches a series row.

| Field          | Default | Purpose                                                                                                                                                                                            |
| -------------- | ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cron`         | unset   | Cron for the scheduled job; absent disables it. The `POST /series/refresh-all` and `tsundoku refresh-series` triggers still work. 5-field crons get padded to seconds-0.                           |
| `batch_size`   | 50      | Max rows refreshed per tick. Each row is one outbound provider call; tune to what the provider's rate limit tolerates. `0` makes every tick a no-op (transient disable without dropping the cron). |
| `min_age_days` | 7       | Skip rows whose `metadata_fetched_at` is fresher than this many days. Matches MangaBaka's published-dump cadence; tighten or loosen by observed upstream churn.                                    |

The full operator-facing story (UI, CLI, endpoints) is on the
[Providers page → Series-row refresh](./providers.md#series-row-refresh).

## `[providers.mangabaka]`

```toml
[providers.mangabaka]
enabled = true
api_base_url           = "https://api.mangabaka.dev"
# api_key              = "mb-..."
# api_fallback         = true
# offline_dump_url     = "https://..."
# offline_refresh_cron = "0 4 * * 0"
negative_cache_ttl_days = 7
timeout_seconds        = 60
```

| Field                     | Purpose                                                                                |
| ------------------------- | -------------------------------------------------------------------------------------- |
| `enabled`                 | When false, provider is skipped at boot.                                               |
| `api_base_url`            | Live-API endpoint. Used for `api_fallback` calls.                                      |
| `api_key`                 | Optional. Without it, the provider runs in offline-only mode.                          |
| `api_fallback`            | When true + `api_key` set, cache misses fall back to a live API call.                  |
| `offline_dump_url`        | URL of the nightly SQLite dump. Leave unset to disable the offline cache.              |
| `offline_refresh_cron`    | Cron expression for periodic dump refresh. 5-field crons are auto-padded to seconds-0. |
| `negative_cache_ttl_days` | How long to remember "this ID isn't in MangaBaka" before retrying.                     |
| `timeout_seconds`         | HTTP timeout for both the API and the dump download.                                   |

The full lifecycle of the offline cache lives on the
[Providers page](./providers.md).

## `[ingestion]`

Controls how releases get matched to series rows. Thresholds use
Dice-coefficient scoring on character bigrams (case- and
punctuation-insensitive).

```toml
[ingestion]
resolution_threshold  = 0.85
review_threshold      = 0.55
fuzzy_search_limit    = 10
queue_low_confidence  = true

# [[ingestion.format_type_rules]]
# formats        = ["cbz", "cbr", "zip", "rar"]
# required_kinds = ["manga", "manhwa", "manhua"]

# [ingestion.cleanup]
# extra_format_keywords = ["Remastered", "DigitalUncen"]
```

### Thresholds

| Field                  | Default | Meaning                                                                                   |
| ---------------------- | ------- | ----------------------------------------------------------------------------------------- |
| `resolution_threshold` | 0.85    | Dice score above which a fuzzy hit auto-resolves.                                         |
| `review_threshold`     | 0.55    | Minimum score to surface a candidate in the review queue.                                 |
| `fuzzy_search_limit`   | 10      | Cap on `MetadataProvider::search` candidates inspected per release.                       |
| `queue_low_confidence` | true    | When false, sub-threshold hits leave the release `unresolved` without writing candidates. |

### `[[ingestion.format_type_rules]]`

A rule fires when any of its `formats` is detected on the release.
The matched series's kind must then be in `required_kinds`
(case-insensitive). Mismatches demote the release to `ambiguous` for
human review. Empty list disables format-type validation.

### `[ingestion.cleanup]`

Title-cleaning knobs. The resolver strips structural noise (parens,
brackets, volume/chapter markers, file extensions, format keywords,
year tokens) from each release title before searching the active
provider. Rules and ordering live in code; the only operator surface
is the keyword list.

`extra_format_keywords` are appended to the built-in list (`Digital`,
`Raw`, `Color`, `Colored`, `Omnibus`, `Premium`, `Complete`,
`Decensored`, `Uncensored`, `Webtoon`, `WN`, `LN`). Each entry is
matched as a whole-word token, case-insensitively. Regex
metacharacters are rejected at config load — entries must be plain
words or phrases.

## `[[sources]]`

Each entry is one polled instance. The `kind` field picks the
implementation; per-kind options live in the matching nested block.

```toml
[[sources]]
kind    = "nyaa"
name    = "english-manga-trusted"
cron    = "0 */2 * * *"     # every 2 hours
enabled = true
  [sources.nyaa]
  feed_url        = "https://nyaa.si/?page=rss&c=3_1&f=2"
  fetch_details   = true
  timeout_seconds = 30
  site_base_url   = "https://nyaa.si"
```

| Field                          | Purpose                                                                  |
| ------------------------------ | ------------------------------------------------------------------------ |
| `kind`                         | Implementation selector. v1 ships only `"nyaa"`.                         |
| `name`                         | Stable identifier. Used in URLs, metrics, and config overrides.          |
| `cron`                         | Schedule. Omit to skip the scheduled poll; the CLI one-shot still works. |
| `enabled`                      | Set to false to keep the entry around without polling.                   |
| `sources.nyaa.feed_url`        | The RSS URL to poll. Tune per the [Sources page](./sources.md).          |
| `sources.nyaa.fetch_details`   | Fetch each post's detail page for richer file/link data. Default true.   |
| `sources.nyaa.timeout_seconds` | HTTP timeout for feed + detail fetches.                                  |
| `sources.nyaa.site_base_url`   | Override for proxied feeds. Defaults to `https://nyaa.si`.               |

Multiple `[[sources]]` blocks polling the same `kind` are supported —
useful for distinct uploader feeds, language subcategories, or
parallel polling at different cadences.

## `[codex]`

Optional integration with a [Codex](https://codex.4sh.dev)
library, powering the admin-only ownership overlay on the feed. Disabled
by default; the whole block may be omitted. See the dedicated
[Codex integration](./codex.md) page for the full picture.

```toml
[codex]
enabled         = true
base_url        = "https://codex.example.com"
api_key         = "codex-reader-api-key"
sync_cron       = "0 */6 * * *"   # every 6 hours; omit to disable the cron
timeout_seconds = 30
```

| Field             | Purpose                                                                                   |
| ----------------- | ----------------------------------------------------------------------------------------- |
| `enabled`         | Master switch. When false, no cron, no API surface, no outbound calls.                    |
| `base_url`        | Codex base URL. **Required when enabled.** Doubles as the deep-link host.                 |
| `api_key`         | **Codex's** `X-API-Key` (Reader scope). **Required when enabled.** Not tsundoku's token.  |
| `sync_cron`       | Schedule for the presence sweep. Omit to disable the cron (manual refresh still works).   |
| `timeout_seconds` | HTTP timeout per outbound Codex request. Default 30.                                       |

`api_key` is unrelated to `[auth]` above — it authenticates tsundoku *to
Codex*, not callers to tsundoku. It can be supplied via the
`TSUNDOKU_CODEX__API_KEY` environment variable instead of the file. When
`enabled = true`, tsundoku refuses to start if `base_url` or `api_key` is
missing.
