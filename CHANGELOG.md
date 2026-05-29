# Changelog

All notable changes to tsundoku will be documented in this file.

## [1.5.3] - 2026-05-29

### Bug Fixes

- *(web)* Render Back-to-feed link via renderRoot to fix type error

## [1.5.2] - 2026-05-29

### Bug Fixes

- *(web)* Preserve feed filters when navigating to a series and back
- *(source)* Stop reading spaced "vNN - subtitle" as a volume/chapter range

### Documentation

- Refresh UI screenshots

## [1.5.1] - 2026-05-29

### Features

- *(web)* Collapse the series tag list with a show-more toggle

### Bug Fixes

- *(source,web)* Parse volume/chapter spans from real-world release names

### Documentation

- Refresh UI screenshots

## [1.5.0] - 2026-05-29

### Features

- *(source,db,resolution,api,web)* Track highest volume/chapter across releases

## [1.4.1] - 2026-05-29

### Features

- *(web)* Add results-per-page selector to the series feed

### Performance

- *(web)* Replace genre/tag chip clouds with searchable multi-selects

## [1.4.0] - 2026-05-29

### Features

- *(scheduler,api,web)* Re-enrich existing releases by status; surface release details on series page

## [1.3.0] - 2026-05-29

### Features

- *(source,api,web)* Capture and display the Nyaa Information-field link

## [1.2.0] - 2026-05-29

### Features

- *(scheduler,api,web)* Split enrich + resolve timings on poll_runs and surface on metrics card

## [1.1.2] - 2026-05-29

### Features

- *(web,api)* Sort series releases by post date and add copy-link buttons
- *(resolution)* Match mangabaka.org links directly instead of falling to fuzzy

## [1.1.1] - 2026-05-29

### Features

- *(resolution)* Add split_chapter_suffix rule for bare trailing chapter numbers

### Refactor

- *(web)* Trim job pill copy and move phase into tooltip

### Documentation

- Capture admin maintenance screenshot and refresh the screenshot set

## [1.1.0] - 2026-05-28

### Bug Fixes

- *(scheduler,api)* Centralize job lock + event lifecycle so manual triggers actually run

## [1.0.3] - 2026-05-28

### Features

- *(web)* Add per-series refresh-metadata button

## [1.0.2] - 2026-05-28

### Features

- *(api,web)* Expose app version via /info and show it in the header

### Bug Fixes

- *(web)* Prevent provider/source card action group from shrinking
- *(web)* Mock /info and /events/jobs in MSW handlers

## [1.0.1] - 2026-05-28

### Features

- *(screenshots)* Add Playwright workflow for docs screenshots
- *(cli,config)* Add init-config command and starter-template bootstrap

### Bug Fixes

- *(makefile)* Touch CHANGELOG.md before git-cliff --prepend

## [1.0.0] - 2026-05-28

### Features

- *(db)* Land storage layer (sea-orm + FTS5) and frontend bootstrap follow-ups
- *(metadata)* Add MetadataProvider abstraction with MangaBaka impl and offline cache
- *(source)* Add DiscoverySource trait and Nyaa source with poll CLI
- *(resolution)* Add release-to-series resolution pipeline with resolve CLI
- *(scheduler)* Add per-source poll and per-provider cache-refresh cron worker
- *(api)* Add v1 HTTP API + single-user auth in td-api crate
- *(web)* Add series feed and detail views with persisted filter presets
- *(web)* Add review queue UI with admin-token auth gate
- *(config)* Auto-merge sibling .local overlay between base file and env
- *(admin)* Add /admin page with source/provider config and fan-out triggers
- *(catalog)* Normalize genres and tags into joinable tables with filter UI
- *(metrics)* Record scheduler runs and surface admin observability dashboards
- *(resolve)* Translate legacy MangaUpdates IDs via live 308 redirects
- *(resolve)* Clean release titles and Dice against cleaned queries
- *(docs)* Scaffold Docusaurus docs site with landing page
- *(docs)* Wire OpenAPI reference + align Makefile with Codex operational shape
- *(review)* Add bulk retry-all button for the review queue
- Rerank MangaBaka FTS hits and enrich the review queue card
- *(admin)* Split admin into route-based multi-page UI under /admin/*
- *(web)* Add debounced search and card/list view toggle on the feed
- *(api,web)* Stream manual-trigger lifecycle via SSE
- *(http)* Per-host outbound rate limiter for nyaa, mangabaka, mangaupdates
- *(http)* Retry on 429/5xx with capped Retry-After and jittered backoff
- *(admin)* Move review queue under /admin/review and fix sidebar active state
- *(review)* Surface provider URL and alternate titles on candidates
- *(api,web)* Show description, genres, and tags on the series feed list view
- *(nyaa)* Walk multiple feed pages and skip already-seen posts
- *(api,web)* Surface max_pages on the discovery-sources admin card
- *(nyaa)* Add backfill command via HTML listing pagination
- *(api,web)* Trigger source backfill in-process via endpoint and admin UI
- *(resolution)* Strip bare chapter ranges and split subtitle heads
- *(api,web)* Add standalone "kept" release disposition
- *(api,web)* Add manual series creation
- *(web)* Link review releases to an existing catalog series
- *(api)* Filter the review queue by title, source, format, and status
- *(api)* Bulk-reject and bulk-retry the review queue
- *(web)* Search and filter the review queue
- *(web)* Multi-select and bulk retry/reject in the review queue
- *(web)* Show details and an expandable description on kept releases
- *(api,web)* Show torrent file list and metadata volume/chapter counts
- *(api,web)* Show series format on review candidates
- *(web)* Show tags on the series page and relink misclassified releases
- *(api,web)* Surface series rating, counts, and release count with matching sorts and filters
- *(resolution,db,config)* Groundwork for bulk series metadata refresh
- *(scheduler,db)* Scheduled series-metadata refresh job
- *(api,cli)* Bulk series-metadata refresh endpoint + CLI
- *(resolution,api)* Filter fuzzy candidates by release format; allow re-resolving resolved rows
- *(api,web)* Multi-select genres and tags with any/all toggle
- *(web)* Add dark mode toggle in the app shell

### Bug Fixes

- *(db)* Derive release id from (source_kind, external_id) only
- *(web)* Prevent provider search results from compressing under flex shrink
- *(nyaa)* Resolve XML entity references in parsed item fields
- *(ci)* Unblock cargo-dist and Dockerfile.cross builds
- *(mangabaka)* Correct public series URL to mangabaka.org/{id}
- *(mangabaka)* Populate genres and tags from offline dump
- *(ci)* Inherit workspace repository key in all crate manifests
- *(ci)* Allow-dirty cargo-dist generated CI files
- *(ci)* Define [profile.dist] for cargo-dist builds
- *(review)* Enlarge candidate cards and show full alternate titles
- *(web)* Forward hasReleases to the series API and tri-state the boolean filters

### Other

- *(api)* Add free-text series search and id-maps metrics endpoint

### Refactor

- *(db)* Drop legacy series.genres_json column
- *(scheduler)* Persist and resolve each release before the next
- *(source)* Move per-release detail fetch into DiscoverySource::enrich
- *(series)* Build provider URLs in the UI from (provider, externalId)

### Documentation

- Expand README into a full operator reference
- *(config)* Document ingestion.cleanup.extra_format_keywords
- *(site)* Migrate operator content from README into eight pages
- Add browse + kept pages, refresh review-queue and series-refresh coverage

### Miscellaneous Tasks

- Bootstrap workspace, config, CLI, frontend, and ops tooling
- Add full CI/build pipelines, pre-commit hooks, and issue templates
- *(docker)* Wire MangaBaka env vars and .env support into compose
- *(web)* Swap to plugin-react with rolldown chunking, polish header

