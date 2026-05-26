# tsundoku

A standalone Rust service that polls manga discovery sources, resolves each
release to a [MangaBaka](https://mangabaka.org) series, and maintains a local
catalog of discoverable series. Browse it through the embedded web UI; gate
the write surface behind a single-user admin token.

Ships as one binary with the React SPA embedded, or as a multi-arch Docker
image. SQLite-only; no Postgres, no Redis, no external queue.

## Status

v1 in progress. Source-pluggable architecture, but only the **Nyaa** discovery
source ships in v1. Metadata-provider-pluggable, but only the **MangaBaka**
provider ships. The architecture is ready for additional sources/providers
without core refactors.

## Quick start

### Run from source

```bash
# 1. Copy the example config and edit it (data_dir, at least one source)
cp config/tsundoku.example.toml config/tsundoku.toml
$EDITOR config/tsundoku.toml

# 2. Apply migrations
cargo run -- migrate

# 3. (Optional but recommended) refresh the MangaBaka offline cache
cargo run -- refresh-metadata

# 4. Serve the API + scheduler
cargo run -- serve
```

The server binds to `127.0.0.1:8080` by default. The Scalar API docs UI is at
[`/docs`](http://127.0.0.1:8080/docs); the embedded SPA is at [`/`](http://127.0.0.1:8080/).

### Run with the embedded frontend

The SPA is embedded into the binary behind a Cargo feature so `cargo check`
and `cargo test` work without `web/dist` existing:

```bash
make build                         # builds web/dist, then cargo build --release --features embed-frontend
./target/release/tsundoku serve
```

### Run with Docker

```bash
# Production-style stack: builds the multi-stage Dockerfile, single service,
# bind-mounts ./config (read-only) and ./docker/data (read-write).
make prod-up                       # or: docker compose --profile prod up -d
make prod-logs                     # tail logs
make prod-down                     # stop
```

The compose file's `prod` profile is what `docker compose up` runs on a clean
checkout once a config file is in `./config/`.

A pre-built image is published to GHCR on every push to `main` and on every
version tag:

```bash
docker pull ghcr.io/<owner>/tsundoku:latest      # main
docker pull ghcr.io/<owner>/tsundoku:v0.1.0      # tagged release
```

## Configuration

Config files live in `config/` and are picked up automatically. TOML and YAML
are both supported (the parser is chosen by extension). Any value can be
overridden by an environment variable using the `TSUNDOKU_` prefix and `__`
for nesting:

```bash
TSUNDOKU_SERVER__PORT=9000 \
TSUNDOKU_STORAGE__DATA_DIR=/var/lib/tsundoku \
  tsundoku serve
```

See [`config/tsundoku.example.toml`](config/tsundoku.example.toml) for every
key with inline documentation. The minimum viable config:

```toml
[server]
host = "127.0.0.1"
port = 8080

[storage]
data_dir = "./data"           # everything on-disk lives under here

[metadata]
active_provider = "mangabaka"

[providers.mangabaka]
enabled = true
api_base_url = "https://api.mangabaka.dev"

[[sources]]
kind = "nyaa"
name = "english-manga"
cron = "0 */2 * * *"          # every 2 hours; 5-field crons get padded to seconds-0
  [sources.nyaa]
  feed_url = "https://nyaa.si/?page=rss&c=3_1&f=2"
```

### Auth

Reads are public by default. To gate them, set `auth.read_requires_auth = true`
and `auth.api_key = "..."`; the frontend sends the key via `X-API-Key` or
`Authorization: Bearer`. Write endpoints (the review queue, manual polls,
manual cache refresh) **always** require `auth.admin_token` as a bearer token,
and return `503 Misconfigured` if `admin_token` is unset.

### Storage layout

`storage.data_dir` is the single on-disk root. Each subpath defaults to a
predictable subdirectory:

| Default                          | Contents                              |
| -------------------------------- | ------------------------------------- |
| `${data_dir}/db/tsundoku.db`     | SQLite database (the only stateful file) |
| `${data_dir}/cache/providers/`   | Metadata provider offline caches (e.g. MangaBaka dump) |
| `${data_dir}/cache/covers/`      | Reserved for future cover-image cache |
| `${data_dir}/tmp/`               | Transient downloads, in-progress ingests |

Docker mounts a single volume at `data_dir`. Back up by copying the directory.

## CLI reference

```
tsundoku <COMMAND> [OPTIONS]
```

Every command accepts `--config <PATH>` (defaults to `config/tsundoku.toml`).

| Command                                | What it does                                                                 |
| -------------------------------------- | ---------------------------------------------------------------------------- |
| `serve`                                | Start the HTTP server + scheduler (per-source poll cron + per-provider refresh cron). |
| `migrate`                              | Apply pending database migrations and exit.                                  |
| `poll [--source NAME]`                 | One-shot poll of every enabled source (or a named one). Persists releases as `unresolved`; resolution runs separately. |
| `resolve [--retry-unresolved]`         | Walk releases that have not been resolved and run them through the resolution pipeline. `--retry-unresolved` also re-runs `ambiguous` rows. |
| `refresh-metadata [--provider ID]`     | Refresh the offline cache for every registered provider (or one). Providers without an offline cache are no-ops. |
| `openapi [--output PATH]`              | Write the OpenAPI specification (default: `web/openapi.json`).               |

The scheduler runs `poll → resolve` automatically inside `serve`; the CLI
variants exist for one-shots, debugging, and ops scripts.

## Make targets

```bash
make help                # full list
make check               # fmt + clippy (-D warnings) + tests
make build               # builds web/dist, then cargo build --release --features embed-frontend
make dev-up              # docker compose dev stack (backend hot reload + Vite)
make prod-up             # docker compose production stack
make openapi-all         # regenerate the OpenAPI spec and the TypeScript types
make changelog           # git-cliff CHANGELOG.md
make release-prepare VERSION=1.0.0
```

## CI / release

Two workflows under [`.github/workflows/`](.github/workflows/):

- [`ci.yml`](.github/workflows/ci.yml) on every PR: partitioned tests
  (cargo-nextest), Rust lint, frontend lint/tests/build, and a multi-arch
  Docker build pushed to GHCR with a PR tag.
- [`build.yml`](.github/workflows/build.yml) on push to `main` and on version
  tags: the same checks plus cargo-dist platform binaries (with build
  provenance attestations) and multi-arch Docker images. Tags also create a
  GitHub Release with the changelog body.

Both publish to `ghcr.io/<owner>/tsundoku`. Repository settings must grant
workflows "Read and write" permission (Settings → Actions → General →
Workflow permissions).

The changelog uses [git-cliff](https://git-cliff.org/) with conventional
commits; see [`cliff.toml`](cliff.toml).

## Pre-commit hooks

```bash
brew install pre-commit          # or: pipx install pre-commit
make setup-hooks                 # installs the hooks
pre-commit run --all-files       # optional: run all hooks once across the repo
```

The hooks run `cargo fmt`, `cargo clippy` (warning-only), the frontend lint
(biome), and the OpenAPI sync check. Bypass for one commit with
`git commit --no-verify` (don't make a habit).

## FAQ

**Why standalone instead of a Codex plugin?**
Codex's release-tracking flow is matched-by-default (alias-driven). tsundoku is
unmatched-by-default: it scans firehoses for series the user has not yet
imported. Bolting that data shape onto Codex would permanently bloat its
schema for a workflow that does not generalize. The `series.owned` column on
`series` is reserved as a future hook; whether and how it gets populated will
depend on what Codex's HTTP API actually exposes when that work happens.

**Why SQLite?**
Single-user, single-author, single-host workload. The biggest live table after
a year of polling is well under a million rows. Postgres would be operational
overhead with no payoff at this scale. If the workload ever crosses
multi-writer territory, `sea-orm`'s `sqlx-postgres` feature is a flag flip.

**Where's the offline MangaBaka dump?**
MangaBaka publishes nightly dumps at
`https://api.mangabaka.dev/v1/database/series.sqlite.tar.gz` (~476 MB
compressed). `tsundoku refresh-metadata` downloads, verifies via SHA-1
sidecar, extracts, and adds indexes + an FTS5 mirror. The extracted dump
lives at `${data_dir}/cache/providers/mangabaka/series.sqlite` and is opened
read-only as a side database.

**Why no SSE / websockets in v1?**
Discovery is a low-frequency activity (polls run on cron, minutes apart).
Frontend uses TanStack Query polling with React Query cache; the operational
cost of a push channel buys nothing here. The plan reserves the right to
revisit if a future phase needs live updates.

**How do I add a new discovery source?**
Implement the [`DiscoverySource`](crates/td-source/src/source.rs) trait in a
new `td-source-<name>` crate, add a `[[sources]] kind = "<name>"` config
schema variant, and register it in the source registry builder. No core
changes. The PRD's "Future Considerations" section has the full step list.

**How do I add a new metadata provider?**
Implement the [`MetadataProvider`](crates/td-metadata/src/provider.rs) trait
in a new `td-metadata-<name>` crate, add a `[providers.<name>]` config block,
and add one line to the registry builder. If the provider has an offline
dump, ship a nested sea-orm migrator inside the provider crate; the top-level
`migration::Migrator` composes it. No core changes.

**Can I run multiple metadata providers at once?**
Multiple providers can be *registered* (so the review UI can search across
them and the resolver can chain foreign-ID lookups), but exactly one is
designated `metadata.active_provider` and runs the auto-resolution path.
Switching active providers is a config-level decision.

## License

MIT OR Apache-2.0
