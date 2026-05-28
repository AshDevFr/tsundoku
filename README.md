# tsundoku

A standalone Rust service that polls manga discovery sources, resolves
each release to a [MangaBaka](https://mangabaka.org) series, and
maintains a local catalog of discoverable series. Browse it through
the embedded web UI; gate the write surface behind a single-user admin
token.

Ships as one binary with the React SPA embedded, or as a multi-arch
Docker image. SQLite-only; no Postgres, no Redis, no external queue.

## Documentation

Full operator documentation lives at
**[tsundoku.4sh.dev](https://tsundoku.4sh.dev)**:

- [Getting Started](https://tsundoku.4sh.dev/docs/getting-started) —
  install + first run, Docker and from-source.
- [Configuration](https://tsundoku.4sh.dev/docs/configuration) — the
  field-by-field reference, generated from
  [`config/tsundoku.example.toml`](config/tsundoku.example.toml).
- [Sources](https://tsundoku.4sh.dev/docs/sources),
  [Providers](https://tsundoku.4sh.dev/docs/providers),
  [Review queue](https://tsundoku.4sh.dev/docs/review-queue) — day-to-day
  operations.
- [Deployment](https://tsundoku.4sh.dev/docs/deployment),
  [Troubleshooting](https://tsundoku.4sh.dev/docs/troubleshooting) —
  prod + diagnostic recipes.
- [Architecture](https://tsundoku.4sh.dev/docs/architecture) — short
  design tour.

While developing locally, the runtime
[Scalar UI](http://127.0.0.1:8080/docs) at `/docs` is the live API
reference.

## Quick start

```bash
# 1. Copy the example config and edit it (data_dir, at least one source).
cp config/tsundoku.example.toml config/tsundoku.toml
$EDITOR config/tsundoku.toml

# 2. Apply migrations.
cargo run -- migrate

# 3. (Optional) refresh the MangaBaka offline cache.
cargo run -- refresh-provider-cache

# 4. Serve the API + scheduler.
cargo run -- serve
```

The server binds to `127.0.0.1:8080` by default. The embedded SPA is
at [`/`](http://127.0.0.1:8080/).

### Docker

```bash
cp config/tsundoku.example.toml config/tsundoku.toml
$EDITOR config/tsundoku.toml
make prod-up                       # docker compose --profile prod up -d
```

Pre-built multi-arch images at `ghcr.io/skewb1k/tsundoku:latest` and
`:v<version>`.

## Status

v1 in progress. Source-pluggable architecture, but only the **Nyaa**
discovery source ships in v1. Metadata-provider-pluggable, but only
the **MangaBaka** provider ships. The architecture is ready for
additional sources and providers without core refactors.

## Contributing

See [`CLAUDE.md`](CLAUDE.md) for the contributor-facing architecture
overview and code conventions. The docs site has a non-contributor
[architecture tour](https://tsundoku.4sh.dev/docs/architecture) for a
gentler intro.

Pre-commit hooks:

```bash
brew install pre-commit          # or: pipx install pre-commit
make setup-hooks
```

## License

MIT OR Apache-2.0
