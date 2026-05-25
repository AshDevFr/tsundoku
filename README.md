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

## Configuration

Copy `config/tsundoku.example.toml` to `config/tsundoku.toml` and edit.
Any value can be overridden via environment variables using the
`TSUNDOKU_` prefix and `__` for nesting
(e.g. `TSUNDOKU_SERVER__PORT=9000`). YAML config files are also supported.
