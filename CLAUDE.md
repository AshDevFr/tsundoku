# tsundoku

Manga discovery service that polls sources and resolves releases to MangaBaka series.

## Project Overview

tsundoku polls one or more discovery sources (Nyaa.si is the v1 source),
resolves each release to a MangaBaka series, and maintains a local catalog of
discoverable series. The browse UI surfaces series the user has not yet
imported into Codex; the review UI lets the operator manually link releases
the auto-resolver couldn't.

It deliberately does **not** track releases for series the user already owns
(Codex's `release-nyaa` plugin already does that), download torrents itself,
post-process or track download progress, reach into Codex's database, or do
anything multi-user. Single-user auth, single host, single SQLite file.

It *does* offer an optional, admin-only **"send to torrent client"** action:
one click pushes a discovered release into the operator's torrent client
(ruTorrent in v1) behind a `DownloadClient` trait. It is off by default,
configured entirely from the `[download]` config block (there is no setup UI,
matching how Codex and sources are configured). The connection is tested at
launch, on demand from the admin Download page, and on an optional
`health_cron`; every send attempt (success or failure) is recorded. tsundoku
still never tracks download *progress* afterward (the torrent client stays the
source of truth for that).

## Architecture

- **Single Rust binary** — axum 0.8 + tokio + sea-orm 1.x, embeds the React SPA via `rust-embed` behind the `embed-frontend` Cargo feature.
- **SQLite only.** Personal-scale volume; no Postgres-only features unless the project commits to Postgres. The pool is pinned to 1 connection so per-connection PRAGMAs (`foreign_keys=ON`, `busy_timeout=5000`) actually stick.
- **Source-pluggable.** `DiscoverySource` trait + per-source crate (`td-source-<name>`). v1 ships only Nyaa.
- **Metadata-provider-pluggable.** `MetadataProvider` trait + registry + per-provider crate (`td-metadata-<name>`). Exactly one provider is designated `metadata.active_provider` and runs the auto-resolution path; others can be registered for cross-provider foreign-ID chains and review UI search. v1 ships only MangaBaka.
- **Resolution pipeline** (in `td-resolution`) runs four steps in order: known-external-id short-circuit → active provider's `resolve_by_foreign_id` → fuzzy title via active provider's `search` (Dice-coefficient on character bigrams) → format-to-kind validation. Below threshold but plausible matches go to a review queue.
- **Scheduler** (`td-scheduler` on top of `tokio-cron-scheduler`) runs per-source poll cron + per-provider cache-refresh cron from `serve`. Per-key mutexes (`dashmap`) prevent overlapping ticks; the API's manual triggers share those locks.
- **Real-time via SSE, not WebSockets.** Currently the frontend just polls via TanStack Query; the SSE channel is reserved for a future phase. The `axum` `ws` feature is intentionally not enabled.
- **Single on-disk root.** `storage.data_dir` holds the SQLite DB, every provider's offline cache, the reserved cover cache, and a tmp scratch area. Docker mounts a single volume; backups are a directory copy.

## Key Design Decisions

- **Standalone, not a Codex plugin.** Discovery is unmatched-by-default and discovery-centric, which is the opposite shape from `release-nyaa`. Bolting it onto Codex would permanently bloat Codex's schema for a workflow that doesn't generalize. Future Codex integration (for the `owned` flag) goes through Codex's HTTP API.
- **Surrogate `series.id` PK + `series_external_ids` mapping table.** Series rows are provider-agnostic; provider IDs live in `series_external_ids` with `UNIQUE(provider, external_id)` and `UNIQUE(series_id, provider)`. Adding or swapping providers does not require a series-table schema change.
- **MangaBaka offline cache is the published dump, opened read-only as a side database** — *not* re-ingested into provider-owned tables. The provider adds 8 source-id indexes + an FTS5 mirror once per refresh; queries run against the dump directly. Re-ingesting 585k rows on every refresh would be gratuitous, and the file is the canonical source — drift would mean we're wrong. The nested-migrator wiring stays in place for future providers that *do* need their own schema.
- **Format-to-kind validation runs *after* a confident match.** A mismatched pair (e.g. CBZ release linked to a `novel` series) is demoted from `resolved` to `ambiguous`; the link is still persisted so the review UI has context.
- **Single-user auth from config, not a users table.** Reads are public by default; flip `auth.read_requires_auth` to gate them with `api_key`. Writes always require `admin_token` as a Bearer; a missing token returns `503 Misconfigured` (distinct from `401 Unauthorized`) so a fresh deploy doesn't look like a credentialing bug.
- **Scheduler locks shared with the API.** The same `Arc<JobLocks>` flows into both `td-scheduler` and `td-api`. A manual `POST /sources/{name}/poll` cannot race a cron tick; both `try_lock` on the per-source mutex and the manual trigger returns `{ triggered: false, skipped: true }` when work is already in flight.
- **Send-to-client is config-only and admin-only, no setup UI.** The torrent client (`[download]` block, ruTorrent in v1) is built once at boot from config behind a `DownloadClient` trait, exactly like the Codex integration — there is deliberately no settings page, since credentials live in config, not a DB, in keeping with single-user-from-config. The send endpoint and the connection-test endpoint live in the `require_admin` writes group, as does `GET /download/status` (which surfaces connection info + live health, never the password). A successful send stamps `sent_to_client_at`/`sent_to_client_label` on the release for a "Sent" badge; the button also appears on the series-detail release rows, not just review/kept.
- **`[download]` holds client-agnostic settings; connection details nest per kind.** `[download.rutorrent]` (`base_url`, `username`, `password`, `url_path`) is where the ruTorrent connection lives, so a second client would add `[download.qbittorrent]` without flattening everyone's keys. tsundoku talks to ruTorrent over **rTorrent XML-RPC** (`system.client_version` / `load.raw_start`) at `url_path` (ruTorrent's httprpc plugin, e.g. `plugins/httprpc/action.php`, or a bare `RPC2` mount — the default), the same wire Prowlarr/Sonarr use and the door to download lifecycle later. A ruTorrent web-UI (`addtorrent.php`) transport was prototyped and dropped: its multipart POST is rejected with `400` by Digest-protected seedbox Apache from `reqwest` (curl and raw sockets succeed against the same endpoint, so it's a reqwest-specific wire quirk), and XML-RPC is the better path regardless. The client runs through one `AuthedHttp` that **auto-negotiates Basic or Digest** auth (seedbox ruTorrent typically demands Digest, which `reqwest` can't do natively, so the Digest response is computed via `digest_auth`).
- **Connection health uses a snapshot + transition-only history, shared by download and Codex.** Following the repo's `provider_cache_state`/`provider_refreshes` idiom, each integration keeps a singleton current-state row (`download_status` / `codex_status`) rewritten on every probe, plus an append-only history (`download_health_checks` / `codex_health_checks`) that gets a row **only** when reachability flips or on a manual test — so a frequent `health_cron` (off by default) leaves an uptime timeline, not one row per tick. The launch probe, the cron, and the admin "Test connection" button all funnel through `record_check` / `record_preflight`. Send attempts (including failures, which used to vanish into a 502) are audited in `download_sends`; tsundoku still does not track download *progress*. The Codex side has the symmetric audit: `codex_sync_runs` appends one row per sweep attempt (cron or manual) with its `outcome` (`success` | `preflight_failed` | `auth_failed` | `error`), the fetched/linked counts on success, and the error otherwise — `codex_status` keeps only the latest snapshot, so this table is what powers the admin "Recent syncs" timeline. `sync_codex::run_tick` takes the `trigger` so the manual refresh and the cron are distinguished in both the reachability history and the sweep history.
- **`embed-frontend` is a Cargo feature, not the default.** `cargo check` and `cargo test` work without `web/dist` existing; release builds use `make build` (which runs the frontend build first) or `cargo build --features embed-frontend`.
- **Pure-workspace `Cargo.toml`.** No root `[package]` or `[[bin]]`; the binary lives at `crates/tsundoku/`. Avoids the dual-role-root gotcha and removes the need to forward feature flags from the root.

## Tech Stack

- Rust (edition 2024) — axum 0.8, tokio, sea-orm 1.x, utoipa 5, tracing
- Figment for layered config (TOML or YAML + env, `TSUNDOKU_` prefix, `__` nesting)
- Clap derive for the CLI (`serve`, `migrate`, `poll`, `resolve`, `refresh-provider-cache`, `refresh-series`, `backfill`, `search`, `openapi`)
- `reqwest` 0.13 (`rustls` + `json` + `stream`) for outbound HTTP
- `tokio-cron-scheduler` for per-source / per-provider crons; `dashmap` for per-key job locks
- React 19 + Vite 8 + TypeScript ~6 + Mantine 9 + Zustand 5 + TanStack Query/Router for the web UI
- `openapi-fetch` + `openapi-typescript` for a fully typed API client (regenerated via `make openapi-all`)
- `rust-embed` ships the built frontend inside the binary (`embed-frontend` feature)
- SQLite with foreign keys ON; pool pinned to 1 connection so per-connection PRAGMAs hold
- Real-time via **SSE**, not WebSockets (currently unused; the frontend polls)

## Repository Layout

```
tsundoku/
├── Cargo.toml                  # pure workspace (no root [package])
├── crates/
│   ├── tsundoku/               # binary crate (CLI dispatch only; thin shell over td-api + the registries)
│   ├── td-api/                 # axum routers + handlers + AppState + rust-embed (embed-frontend feature)
│   ├── td-config/              # Figment loader + AppConfig types
│   ├── td-db/                  # sea-orm entities + repos + pool (single-connection SQLite)
│   ├── td-metadata/            # MetadataProvider trait + canonical types + registry
│   ├── td-metadata-mangabaka/  # MangaBaka provider impl + offline-dump ingest
│   ├── td-source/              # DiscoverySource trait + DiscoveredRelease DTO + registry + format detector
│   ├── td-source-nyaa/         # Nyaa RSS parser + post detail HTML parser + external-link extractor
│   ├── td-resolution/          # resolution pipeline (known-id → foreign-id → fuzzy → format validation)
│   ├── td-download/            # DownloadClient trait + ruTorrent XML-RPC client (Basic/Digest auth)
│   └── td-scheduler/           # per-source poll cron + per-provider refresh cron, shared JobLocks
├── migration/                  # sea-orm-migration (top-level by sea-orm convention)
├── config/                     # tsundoku.example.toml (+ .docker.toml for the dev container)
├── web/                        # React 19 + Vite 8 + Mantine 9 SPA (built into web/dist, embedded)
├── Makefile
├── Dockerfile                  # multi-stage production image (musl + alpine runtime)
├── Dockerfile.cross            # ARM64/AMD64 cross-compile variant (no QEMU emulation of rustc)
├── Dockerfile.dev              # cargo-watch hot-reload variant for the dev compose profile
├── docker-compose.yml          # prod + dev profiles
├── compose.watch.yml           # docker compose watch overlay (auto-sync source)
├── compose.codex.yml           # example overlay for colocating with Codex (no shared state in v1)
└── cliff.toml                  # git-cliff conventional-commits config
```

## Development Guidelines

- The binary should be understandable end-to-end. Resist abstractions you do not yet need.
- SQLite is the default. Do not introduce Postgres-only features unless the project commits to Postgres.

### Testing

Use TDD when applicable: write a failing test, then implement.

- **Unit tests** (`#[test]`): pure logic, no I/O.
- **DB tests** (`#[tokio::test]`): in-memory SQLite.
- **HTTP fixture tests**: record real responses under `tests/fixtures/` and replay them. Do not hit live services in tests.
- **Frontend tests**: vitest + Testing Library; MSW for API mocking.

Run only the tests related to your change during development; run the full suite at the end of a phase.

### Rust Conventions

- Edition 2024. New crates use `edition = { workspace = true }`.
- `anyhow` for application errors, `thiserror` for library errors.
- `tracing` for structured logging (not `log` or `println!`).
- `utoipa` for OpenAPI; Scalar UI at `/docs`.
- `#[serde(rename_all = "camelCase")]` on all API DTOs.
- Run `make fmt` and `make lint` (zero warnings) before considering work complete.

### Frontend Conventions

- API types are generated from the OpenAPI spec via `openapi-typescript` (`make openapi-all`). Never hand-edit `src/types/api.generated.ts`.
- Biome for lint + format.

### Naming

- Library crate prefix is `td-`.

### Post-Implementation Checklist

| What changed | Commands |
|---|---|
| Rust backend / DB / migrations | `make fmt` → `make lint` → `make test` |
| Frontend code | `make frontend-lint` → `make test-frontend` |
| OpenAPI-affecting changes (DTOs, handlers, routes) | `make openapi-all` |
| Pre-commit / pre-PR | `make check` |
