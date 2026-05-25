# tsundoku

Manga discovery service that polls sources and resolves releases to MangaBaka series.

## Quick start

```bash
# Backend (serves JSON at /api/v1, Scalar docs at /docs)
cargo run -- serve

# Frontend dev server (proxies /api to the backend, HMR)
cd web && npm install && npm run dev
```

Then open the Vite dev server (http://localhost:5173). For a production-style
single binary with the SPA embedded:

```bash
make build            # builds web/dist, then `cargo build --release --features embed-frontend`
./target/release/tsundoku serve
```

## Common commands

```bash
make help             # list all targets
make check            # fmt + clippy + tests
make openapi-all      # regenerate the OpenAPI spec and the TypeScript types
make dev-up           # docker compose dev stack (backend hot reload + Vite)
make release-prepare VERSION=1.0.0
```

## Git pre-commit hooks

Uses the [pre-commit](https://pre-commit.com/) framework. Install once, then
it runs `cargo fmt`, `cargo clippy` (warning-only), frontend lint (biome), and
the OpenAPI sync check on every `git commit`.

```bash
brew install pre-commit    # or: pipx install pre-commit
make setup-hooks           # runs `pre-commit install`
pre-commit run --all-files # optional: run all hooks once across the repo
```

Bypass for a specific commit with `git commit --no-verify` (don't make a habit).

## CI / Build

Two workflows under `.github/workflows/`:

- `ci.yml` runs on every PR: tests (partitioned with cargo-nextest), Rust lint,
  frontend lint / tests / build, and a multi-arch Docker build pushed to GHCR
  with a PR tag.
- `build.yml` runs on push to `main` and on version tags: the same checks, then
  cargo-dist builds platform binaries (with build provenance attestations) and
  pushes multi-arch Docker images. Version tags also create a GitHub Release.

Both publish to `ghcr.io/<owner>/tsundoku`. Make sure the repository has
"Read and write" workflow permissions enabled under Settings → Actions.

## Configuration

Copy `config/tsundoku.example.toml` to `config/tsundoku.toml` and edit.
Any value can be overridden via environment variables using the
`TSUNDOKU_` prefix and `__` for nesting
(e.g. `TSUNDOKU_SERVER__PORT=9000`). YAML config files are also supported.
