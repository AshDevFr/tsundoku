# tsundoku docs site

Static documentation site for tsundoku, built with
[Docusaurus 3](https://docusaurus.io/). Deployed to Cloudflare Pages
via the repo's GitHub integration — no CI workflow required.

## Local development

```bash
make docs           # installs deps + starts the dev server on :3000
```

Equivalent to:

```bash
cd docs
npm install
npm run start
```

Most changes hot-reload without restarting the server. Markdown,
sidebar config, and component edits all live-update.

## Build

```bash
make docs-build     # produces docs/build/
```

The output is a fully-static site servable from any CDN. Cloudflare
Pages picks this up on every push to `main`.

## Deployment (Cloudflare Pages)

One-time setup, performed in the Cloudflare dashboard:

| Setting | Value |
|---|---|
| Production branch | `main` |
| Build command | `cd docs && npm ci && npm run build` |
| Build output directory | `docs/build` |
| Environment variable | `NODE_VERSION=20` |
| Root directory | `/` (repo root) |

Cloudflare auto-deploys on push to `main`. Preview deploys land on
pull requests. No GitHub Actions YAML needed.

## Layout

```
docs/
├── docusaurus.config.ts    # site config (title, navbar, plugins)
├── sidebars.ts             # docs sidebar shape
├── src/
│   ├── pages/index.tsx     # landing page (hero + features)
│   ├── components/
│   │   └── HomepageFeatures/
│   └── css/custom.css      # palette overrides
├── docs/                   # markdown content (this is what `/docs/*` serves)
│   └── intro.md
└── static/img/             # logos, favicon, social card
```

## Adding a page

1. Create a markdown file under `docs/docs/`, e.g. `configuration.md`.
2. Add it to `sidebars.ts` under the appropriate category.
3. Cross-link from other pages with relative paths
   (`[Configuration](./configuration.md)`); broken links fail the build
   because `onBrokenLinks: 'throw'` is set.

## Phase status

- **Phase 1 (this)** — Scaffold + landing page + minimal docs tree. ✅
- **Phase 2** — Migrate the README's operator content into structured
  pages with proper navigation.
- **Phase 3** — Wire `docusaurus-plugin-openapi-docs` against
  `web/openapi.json` so the static API reference regenerates on every
  `make openapi-all`. Configure Cloudflare Pages.

See `.tsundoku/docs/specs/feature-3-docs-site.md` (gitignored, local
plans) for the full plan.
