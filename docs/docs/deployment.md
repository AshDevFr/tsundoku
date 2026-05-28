---
title: Deployment
sidebar_position: 8
---

# Deployment

tsundoku is designed to deploy as a single container or single binary
sitting alongside Codex. SQLite-only, no external services, no message
queue, no Redis. Back up by copying one directory.

## Docker (recommended)

Pre-built multi-arch images are published to GHCR on every push to
`main` and on every version tag:

```bash
docker pull ghcr.io/skewb1k/tsundoku:latest      # main
docker pull ghcr.io/skewb1k/tsundoku:v0.1.0      # tagged release
```

### Compose

The repo ships a `docker-compose.yml` with `prod` and `dev` profiles.

```bash
make prod-up                       # docker compose --profile prod up -d
make prod-logs                     # tail logs
make prod-down                     # stop
```

The `prod` service bind-mounts:

- `./config` → `/app/config` (read-only) — your `tsundoku.toml`.
- `./docker/data` → `/app/data` (read-write) — SQLite, MangaBaka
  dump, scratch.

Override the data path with `TSUNDOKU_STORAGE__DATA_DIR=/some/path` if
the default layout doesn't fit your host filesystem.

### Custom build

```bash
make docker-build           # local single-arch
make docker-push            # multi-arch via buildx
```

## Standalone binary

Cargo-dist publishes platform binaries on every version tag with build
provenance attestations. Grab one from the
[releases page](https://github.com/skewb1k/tsundoku/releases) or build
from source:

```bash
make build              # builds web/dist, then cargo build --release --features embed-frontend
./target/release/tsundoku serve
```

The `embed-frontend` feature compiles the React SPA into the binary
via `rust-embed`. Without the feature, the API still runs but `/`
returns `404` — useful when running the SPA out-of-process via the
Vite dev server.

## Reverse proxy

tsundoku doesn't terminate TLS or do CORS — front it with a reverse
proxy.

### Caddy

```caddyfile
tsundoku.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

That's it. Caddy auto-provisions a Let's Encrypt cert.

### Nginx

```nginx
server {
    listen 443 ssl http2;
    server_name tsundoku.example.com;

    ssl_certificate     /etc/letsencrypt/live/tsundoku.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/tsundoku.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

### Cloudflare Tunnel

Works as-is. No special config required — tsundoku doesn't look at
proxy headers and doesn't care about source IPs.

## Codex sidecar

When deploying alongside Codex, point both at the same host but
different ports. They don't share a database, don't share auth, and
talk to each other (if at all) over HTTP only.

A skeleton `compose.codex.yml` overlay exists in the repo as an
example. Mount it on top of the prod compose with:

```bash
docker compose -f docker-compose.yml -f compose.codex.yml --profile prod up -d
```

Future versions may integrate via Codex's HTTP API to populate the
`series.owned` flag (so resolved-and-owned series can be auto-hidden
from the catalog). For now, the two services are independent.

## Backups

Everything stateful lives under `storage.data_dir`. To back up:

```bash
systemctl stop tsundoku            # or: docker compose down
tar czf tsundoku-backup-$(date +%Y%m%d).tar.gz /var/lib/tsundoku/
systemctl start tsundoku
```

SQLite supports online backups (`.backup` command) if you can't
afford the downtime, but the consistent file copy is simpler and the
service restarts in seconds.

## CI / release pipeline

Two GitHub Actions workflows:

- [`ci.yml`](https://github.com/skewb1k/tsundoku/blob/main/.github/workflows/ci.yml)
  on every PR: partitioned tests (`cargo-nextest`), Rust lint, frontend
  lint/tests/build, and a multi-arch Docker build pushed to GHCR with
  a PR tag.
- [`build.yml`](https://github.com/skewb1k/tsundoku/blob/main/.github/workflows/build.yml)
  on push to `main` and on version tags: the same checks plus
  `cargo-dist` platform binaries (with provenance attestations) and
  multi-arch Docker images. Tags also create a GitHub Release with
  the changelog body.

Repository settings must grant workflows "Read and write" permission
(Settings → Actions → General → Workflow permissions).

The changelog uses [git-cliff](https://git-cliff.org/) with
conventional commits.

## Pre-commit hooks (contributors)

```bash
brew install pre-commit          # or: pipx install pre-commit
make setup-hooks                 # installs the hooks
pre-commit run --all-files       # optional: run all hooks once
```

The hooks run `cargo fmt`, `cargo clippy` (warning-only), the frontend
lint (Biome), and the OpenAPI sync check. Bypass for one commit with
`git commit --no-verify`.
