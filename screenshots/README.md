# tsundoku Screenshot Automation

Playwright-based screenshot capture for the docs site and marketing.
Runs the published image (`ghcr.io/ashdevfr/tsundoku:latest`) against an
ephemeral SQLite volume, drives a one-shot Nyaa poll to surface real
data, then walks the UI capturing the interesting pages.

## Quick start

```bash
make screenshots
```

Output lands in `screenshots/output/`. To copy the lot into the docs
site, run:

```bash
make screenshots-move-to-docs
```

## Make targets

| Target | What it does |
|---|---|
| `make screenshots` | Full workflow: up → capture → down |
| `make screenshots-fresh` | `screenshots-clean` + `down -v` + `screenshots` |
| `make screenshots-up` | Start the screenshot stack (detached) |
| `make screenshots-down` | Stop the stack |
| `make screenshots-down-v` | Stop and remove the ephemeral DB volume |
| `make screenshots-run` | Run the capture script against an already-running stack |
| `make screenshots-logs` | Tail logs from both containers |
| `make screenshots-shell` | Open a shell in the Playwright container |
| `make screenshots-clean` | Wipe `screenshots/output/` |
| `make screenshots-move-to-docs` | Copy output into `docs/static/img/screenshots/` |

## How it works

`docker compose --profile screenshots` brings up two services on the
shared `tsundoku-net`:

1. **`tsundoku-screenshots`** — the prebuilt backend image with the
   embedded SPA, pointing at `config/tsundoku.screenshots.toml`. The
   SQLite DB lives in an ephemeral named volume so each run starts
   from a clean slate. Override the image with
   `TSUNDOKU_IMAGE=tsundoku:latest` to capture against a local build.
2. **`playwright`** — `mcr.microsoft.com/playwright:noble` with the
   capture script mounted from `./screenshots`. Boots Chromium, seeds
   the admin token into localStorage, triggers a Nyaa poll over HTTP,
   waits for the resolver to drain, then walks the scenarios.

## Scenarios

Each file in `scripts/scenarios/` captures one logical surface. New
scenarios get picked up automatically when added to the `entries` array
in `scripts/capture.ts`.

| File | Captures |
|---|---|
| `browse.ts` | `/` feed (cards + list variant) and the filter panel |
| `series-detail.ts` | `/series/{id}` for the first series on the feed |
| `admin-overview.ts` | `/admin` dashboard |
| `admin-review.ts` | `/admin/review` queue |
| `admin-kept.ts` | `/admin/kept` page |
| `admin-sources.ts` | `/admin/sources` list and per-source detail |
| `admin-providers.ts` | `/admin/providers` list and per-provider detail |
| `admin-metrics.ts` | `/admin/metrics` charts |
| `admin-id-maps.ts` | `/admin/id-maps` |

## Environment variables

Tunable via either the compose service env or the surrounding shell:

| Variable | Default | Effect |
|---|---|---|
| `TSUNDOKU_IMAGE` | `ghcr.io/ashdevfr/tsundoku:latest` | Backend image |
| `SCREENSHOTS_ADMIN_TOKEN` | `screenshots-admin-token` | Admin token (must match between backend + Playwright) |
| `POLL_ON_START` | `true` | Trigger a Nyaa poll before capture. Set `false` for empty-state runs |
| `POLL_SOURCES` | (every enabled source) | CSV of source names to poll |
| `POLL_WAIT_MIN_SECONDS` | `180` | Minimum soak after triggering polls — covers MangaBaka enrichment + cover fetches that linger after the resolver drains |
| `POLL_WAIT_MAX_SECONDS` | `300` | Hard ceiling on the soak window |
| `BASE_URL` | `http://tsundoku-screenshots:8080` | Backend URL the Playwright container hits |
| `VIEWPORT_WIDTH` / `VIEWPORT_HEIGHT` | `1440x900` | Viewport |
| `COLOR_SCHEME` | `dark` | `dark` or `light` |

## Adding a new scenario

1. Create `scripts/scenarios/<name>.ts` exporting `run(page, context)`.
2. Add it to the `entries` array in `scripts/capture.ts`.
3. Use `captureScreenshot(page, "subdir/name")` and the helpers in
   `scripts/utils/wait.ts` — toast dismissal and dir creation are
   handled for you.

## Troubleshooting

- **Browse is empty**: the Nyaa poll either failed or hadn't resolved
  anything before `POLL_WAIT_MAX_SECONDS`. Inspect with
  `make screenshots-logs`; raise the timeout or set
  `TSUNDOKU_PROVIDERS__MANGABAKA__API_KEY` in `.env` to enable the live
  API fallback.
- **Admin pages show the login card**: the admin token didn't get
  seeded into localStorage. Check that `SCREENSHOTS_ADMIN_TOKEN` is
  identical on both the backend (`TSUNDOKU_AUTH__ADMIN_TOKEN`) and the
  Playwright container (`ADMIN_TOKEN`).
- **`port 8082 already in use`**: another container is already
  bound — usually a real tsundoku prod instance. Either stop it or
  remap the `tsundoku-screenshots` port in `docker-compose.yml`.
