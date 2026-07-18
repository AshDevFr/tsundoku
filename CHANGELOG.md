# Changelog

All notable changes to tsundoku will be documented in this file.

## [1.15.2] - 2026-07-18

### Testing

- *(web)* Give the heavy select-all-matching review test an explicit timeout

## [1.15.1] - 2026-07-18

### Bug Fixes

- *(web)* Let the sticky filter panel scroll when taller than the viewport

## [1.15.0] - 2026-06-23

### Features

- *(search)* Optionally match series descriptions

## [1.14.3] - 2026-06-23

### Bug Fixes

- *(codex)* Confirm currency from any comparable axis

## [1.14.2] - 2026-06-23

### Bug Fixes

- *(metadata)* Parse MangaBaka source-id lookup envelope

## [1.14.1] - 2026-06-17

### Bug Fixes

- *(web)* Make series-detail release rows usable on mobile

## [1.14.0] - 2026-06-17

### Features

- *(web)* Make the top navigation mobile-friendly with a burger drawer
- *(web)* Collapse admin nav and feed filters into left drawers on mobile
- *(web)* Full-screen modals, 2-up cards, and wrapped config values on mobile

## [1.13.1] - 2026-06-15

### Features

- *(series)* Add a full-drain mode to the manual series-metadata refresh

## [1.13.0] - 2026-06-15

### Features

- *(web)* Add Nyaa search, clickable badges and bulk send on series detail
- *(web)* Show a detail tooltip on hovering a feed card title
- *(web)* Enrich and enlarge MangaBaka search result cards
- *(web)* Support shift-click range select on series release bulk-send
- *(series)* Carry MangaBaka publication dates and sort the feed by them

## [1.12.0] - 2026-06-14

### Features

- *(series)* Add admin wishlist flag, toggle and from-provider endpoints
- *(web)* Surface the series wishlist with a clip toggle, indicator and filter
- *(web)* Add the wishlist page, MangaBaka add modal and app-bar shortcut
- *(series)* Add an admin source filter to the series list

## [1.11.2] - 2026-06-14

### Features

- *(web)* Make the feed kind and status filters multi-select

## [1.11.1] - 2026-06-09

### Bug Fixes

- *(resolution)* Commit release link and coverage recompute atomically

## [1.11.0] - 2026-06-09

### Features

- *(series)* Add admin catalog export endpoint
- *(web)* Add admin catalog export page
- *(series)* Finish catalog export — OpenAPI, docs, filter polish
- *(spans)* Detect gap-preserving volume/chapter coverage
- *(series)* Track merged release coverage and a change timestamp
- *(series)* Add a cursor-paginated release feed endpoint
- *(series)* Surface volume/chapter coverage in the catalog export
- *(series)* Filter the release feed by external ids via POST

### Bug Fixes

- *(series)* Correct the OpenAPI path for the release feed

### Documentation

- *(series)* Document the externalIds pattern on the feed filter
- *(series)* List the full provider token set on the feed filter

### Miscellaneous Tasks

- *(docker)* Shift host ports off Codex's defaults

## [1.10.2] - 2026-06-05

### Features

- *(series)* Add sort by rating to the browse list

### Documentation

- Add Download and Codex admin pages with screenshots
- Refresh screenshots and add a second Nyaa source to the fixture config

## [1.10.1] - 2026-06-04

### Bug Fixes

- *(resolution)* Resolve against the Nyaa "Information" link, not just description links

## [1.10.0] - 2026-06-04

### Features

- *(series)* Add ignore_completion column for Codex tracking opt-out
- *(series)* Add Codex "ignored" status for completion-tracking opt-out
- *(series)* Add ignore-completion toggle endpoint
- *(web)* Add ignore-completion toggle, badge, and feed filter
- *(download)* Add td-download crate with DownloadClient trait + ruTorrent client
- *(download)* Add config, sent-to-client columns, and AppState client wiring
- *(download)* Add admin send-to-client and status endpoints
- *(web)* Add admin send-to-client button, override popover, and Sent badge
- *(web)* Allow multiple Codex statuses in the feed filter
- *(download)* Add connection-status, health-history, and send-audit tables
- *(download)* Add connection test, status probe, and send-attempt recording
- *(codex)* Add reachability history and on-demand connection test
- *(web)* Add download client admin page and connection-test UI
- *(download)* Add XML-RPC transport, nest [download.rutorrent] config, broaden UI
- *(web)* Dedupe filter presets by name and confirm overwrites
- *(admin)* Add Codex sweep history and richer download send audit

### Bug Fixes

- *(web)* Stop dropping spaces while typing in the filter search
- *(download)* Support HTTP Digest auth for ruTorrent, not just Basic
- *(mangabaka)* Validate dump content instead of trusting upstream sha1 sidecar

### Refactor

- *(download)* Drop the unused ruTorrent web-UI transport, XML-RPC only

### Documentation

- *(readme)* Add project status & support section

## [1.9.0] - 2026-06-01

### Features

- *(series)* Add PATCH endpoint to edit manual series
- *(series)* Add manual/auto metadataSource filter to the list endpoint
- *(web)* Edit manual series + manual/auto feed filter

### Documentation

- *(browse)* Document manual-series editing and the source filter

## [1.8.2] - 2026-05-31

### Bug Fixes

- *(review)* Keep the group list from scrolling horizontally

## [1.8.1] - 2026-05-31

### Bug Fixes

- *(review)* Cap the group list height and refresh it after a decision

## [1.8.0] - 2026-05-31

### Features

- *(review)* Add searchQuery + breadth group filter to the review queue
- *(review)* Add release grouping to the review queue
- *(review)* Add a release-grouping panel to the review queue
- *(review)* Surface a dominant-candidate hint on release groups
- *(review)* Make group rows full-width and auto-collapse on select

## [1.7.2] - 2026-05-31

### Features

- *(review)* Add a per-page size selector to the queue

## [1.7.1] - 2026-05-31

### Features

- *(review)* Shift+click to select a range of releases in the queue

## [1.7.0] - 2026-05-31

### Features

- *(review)* Sort by title, collapsible cards, and bulk assign-to-series
- *(review)* Resolve foreign provider IDs and surface comment-sourced link suggestions

### Bug Fixes

- *(web)* Redirect unknown routes to the feed instead of dead-ending

### Documentation

- *(codex)* Add a Codex dashboard screenshot to the integration page
- Add MIT and Apache-2.0 license files

## [1.6.2] - 2026-05-30

### Features

- *(web)* Re-enrich multiple sources or all at once

### Documentation

- Document the wide layout mode and refresh UI screenshots

## [1.6.1] - 2026-05-30

### Features

- *(web)* Rework Codex ownership badges to highlight the actionable state
- *(web)* Persist display preferences and add a wide layout mode

### Documentation

- *(codex)* Document admin-only ownership badges and refresh screenshots

### Miscellaneous Tasks

- *(screenshots)* Capture wide layout and a real filtered feed state

## [1.6.0] - 2026-05-30

### Features

- *(config,db,codex)* Add Codex presence foundation
- *(codex,scheduler,api)* Sync job, cron, and admin endpoints for presence overlay
- *(api)* Admin-only Codex presence overlay on series list and detail
- *(web)* Codex presence badges, filter, and connection panel
- *(codex)* Startup status, fetched count, version warning, badge accents

### Documentation

- *(codex)* Document the Codex integration

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

