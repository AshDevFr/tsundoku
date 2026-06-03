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

Pre-built multi-arch images at `ghcr.io/ashdevfr/tsundoku:latest` and
`:v<version>`.

## Status

v1 in progress. Source-pluggable architecture, but only the **Nyaa**
discovery source ships in v1. Metadata-provider-pluggable, but only
the **MangaBaka** provider ships. The architecture is ready for
additional sources and providers without core refactors.

## Project status & support

tsundoku is a solo side project. I built it for my own use and continue to
develop it because I enjoy it. A few things that follow from that:

- **No SLA.** I read everything but respond when I have time.
- **Bug reports are welcome.** Use the issue template and include version,
  deployment method, and relevant logs.
- **Feature requests are welcome, but I will close ones that fall outside
  the scope** (see [Status](#status) above).
- **PRs are welcome** for bugs and small features. For larger changes,
  please open an issue first so we can agree on direction before you write
  code.
- **I don't provide installation support.** The
  [docs](https://tsundoku.4sh.dev/) cover deployment. If you can't get past
  the docs, this project may not be a good fit yet.

## Contributing

The docs site has an
[architecture tour](https://tsundoku.4sh.dev/docs/architecture) covering
the overall design and the source/provider plugin model.

Pre-commit hooks:

```bash
brew install pre-commit          # or: pipx install pre-commit
make setup-hooks
```

## License

Licensed under either of

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([`LICENSE-MIT`](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
