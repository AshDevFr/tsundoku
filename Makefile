.PHONY: help \
	build build-release run watch \
	fmt lint check ci test test-fast \
	frontend frontend-build frontend-install frontend-outdated \
	frontend-lint frontend-lint-fix frontend-mock frontend-mock-fresh test-frontend \
	openapi openapi-types openapi-all \
	docs docs-install docs-outdated docs-start docs-start-fresh \
	docs-build docs-build-fresh docs-serve docs-clear \
	docs-refresh-api-docs docs-gen-api-docs docs-clean-api-docs \
	dev-up dev-up-d dev-up-build dev-down dev-down-v dev-watch dev-check \
	dev-logs dev-logs-backend dev-logs-frontend \
	dev-restart dev-restart-backend dev-restart-frontend \
	dev-shell dev-shell-frontend \
	prod-up prod-up-d prod-down prod-logs \
	docker-build docker-build-clean-cache docker-run docker-push \
	changelog changelog-unreleased changelog-release release-prepare \
	dist-install dist-plan dist-build dist-build-local \
	clean clean-docker clean-all setup-hooks

# Colors
BLUE := \033[0;34m
GREEN := \033[0;32m
YELLOW := \033[0;33m
NC := \033[0m

help: ## Show this help message
	@echo "$(BLUE)tsundoku Development Commands$(NC)"
	@echo ""
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  $(GREEN)%-22s$(NC) %s\n", $$1, $$2}'

# ── Local build & run ────────────────────────────────────────────────────────

build: ## Build the release binary with the embedded SPA (builds frontend first)
	cd web && npm run build
	cargo build --release --features embed-frontend

build-release: build ## Alias for `build`

run: ## Run the server locally (frontend served by Vite separately in dev)
	cargo run -- serve

watch: ## Run with hot reload (requires cargo-watch)
	cargo watch -x 'run -- serve'

# ── Code quality ─────────────────────────────────────────────────────────────

fmt: ## Format Rust code
	cargo fmt --all

lint: ## Run clippy with warnings as errors
	cargo clippy --workspace --all-targets -- -D warnings

test: ## Run backend tests
	cargo test --workspace

test-fast: ## Run backend tests with cargo-nextest
	cargo nextest run --workspace

check: fmt lint test ## Format, lint, and test

ci: ## CI checks: format check, clippy, tests
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace

# ── Frontend ─────────────────────────────────────────────────────────────────

frontend: ## Start the Vite dev server (proxies /api to the backend)
	cd web && npm run dev

frontend-build: ## Build the frontend into web/dist
	cd web && npm run build

frontend-install: ## Install frontend dependencies
	cd web && npm install

frontend-outdated: ## Check for outdated frontend dependencies
	cd web && npm outdated

frontend-lint: ## Lint the frontend (biome)
	cd web && npm run lint

frontend-lint-fix: ## Lint and auto-fix the frontend
	cd web && npm run lint:fix

frontend-mock: ## Start the SPA with MSW mocks (no backend needed)
	cd web && npm run dev:mock

frontend-mock-fresh: openapi-all ## Regenerate API types, then start the SPA with mocks
	cd web && npm run dev:mock

test-frontend: ## Run frontend tests
	cd web && npm run test:run

# ── Docs site (Docusaurus) ───────────────────────────────────────────────────
# Two layers of targets:
#   - docs-*           : direct Docusaurus operations (assume deps installed)
#   - docs-*-fresh     : refresh the OpenAPI reference first, then run the
#                        Docusaurus command. Use after a backend API change.
# The bare `docs` target is a convenience alias for `docs-start`.

docs: docs-start ## Alias for `docs-start`

docs-install: ## Install docs site dependencies
	cd docs && npm install

docs-outdated: ## Check for outdated docs site dependencies
	cd docs && npm outdated

docs-start: ## Start the docs dev server (http://localhost:3000)
	cd docs && npm run start

docs-start-fresh: docs-refresh-api-docs docs-start ## Refresh the API reference, then start the dev server

docs-build: ## Build the docs site into docs/build (Cloudflare Pages publishes this)
	cd docs && npm run build

docs-build-fresh: docs-refresh-api-docs docs-build ## Refresh the API reference, then do a production build

docs-serve: ## Serve the built docs site locally (after `docs-build`)
	cd docs && npm run serve

docs-clear: ## Clear the docs site cache (.docusaurus/, build/)
	cd docs && npm run clear

docs-refresh-api-docs: docs-clean-api-docs docs-gen-api-docs ## Regenerate the OpenAPI reference from the live spec

docs-gen-api-docs: openapi ## Regenerate web/openapi.json, then generate the API reference pages
	cd docs && npm run gen-api-docs

docs-clean-api-docs: ## Remove the generated OpenAPI reference pages
	cd docs && npm run clean-api-docs

# ── OpenAPI ──────────────────────────────────────────────────────────────────

openapi: ## Generate the OpenAPI spec from the backend
	cargo run -- openapi --output web/openapi.json
	@echo "$(GREEN)OpenAPI spec written to web/openapi.json$(NC)"
	@# Copy into docs/ so Cloudflare Pages (root = docs/) can read
	# the spec without checking out the whole monorepo. Both files
	# are committed; the pre-commit OpenAPI sync check keeps them
	# from drifting.
	@mkdir -p docs/api
	@cp web/openapi.json docs/api/openapi.json
	@echo "$(GREEN)OpenAPI spec copied to docs/api/openapi.json$(NC)"

openapi-types: ## Generate TypeScript types from the OpenAPI spec
	cd web && npm run generate:types

openapi-all: openapi openapi-types ## Generate the spec and the TypeScript types

# ── Docker Compose (dev / prod profiles) ─────────────────────────────────────

dev-up: ## Start the dev stack (backend hot reload + Vite)
	docker compose --profile dev up

dev-up-d: ## Start the dev stack (detached)
	docker compose --profile dev up -d

dev-up-build: ## Start the dev stack, rebuilding images
	docker compose --profile dev up --build

dev-down: ## Stop the dev stack
	docker compose --profile dev down

dev-down-v: ## Stop the dev stack and remove volumes
	docker compose --profile dev down -v

dev-logs: ## Tail dev stack logs (backend + frontend)
	docker compose --profile dev logs -f tsundoku-dev frontend-dev

dev-logs-backend: ## Tail backend logs only
	docker compose --profile dev logs -f tsundoku-dev

dev-logs-frontend: ## Tail frontend logs only
	docker compose --profile dev logs -f frontend-dev

dev-restart: ## Restart all dev containers
	docker compose --profile dev restart tsundoku-dev frontend-dev

dev-restart-backend: ## Restart the backend container only
	docker compose --profile dev restart tsundoku-dev

dev-restart-frontend: ## Restart the frontend container only
	docker compose --profile dev restart frontend-dev

dev-watch: ## Start the dev stack with docker compose watch (auto-sync)
	docker compose -f docker-compose.yml -f compose.watch.yml --profile dev watch

dev-shell: ## Open a shell in the dev backend container
	docker exec -it tsundoku-dev sh

dev-shell-frontend: ## Open a shell in the dev frontend container
	docker exec -it tsundoku-frontend-dev sh

dev-check: ## Audit local toolchain (cargo, node, docker, optional speed-ups)
	@echo "$(BLUE)Checking development tools...$(NC)"
	@echo ""
	@echo "$(BLUE)Required:$(NC)"
	@command -v cargo >/dev/null 2>&1 && echo "  $(GREEN)✓ cargo$(NC)" || echo "  $(YELLOW)✗ cargo (install from https://rustup.rs)$(NC)"
	@command -v node >/dev/null 2>&1 && echo "  $(GREEN)✓ node$(NC)" || echo "  $(YELLOW)✗ node (install from https://nodejs.org)$(NC)"
	@command -v docker >/dev/null 2>&1 && echo "  $(GREEN)✓ docker$(NC)" || echo "  $(YELLOW)✗ docker (install Docker Desktop)$(NC)"
	@echo ""
	@echo "$(BLUE)Optional (faster builds):$(NC)"
	@command -v mold >/dev/null 2>&1 && echo "  $(GREEN)✓ mold$(NC) (faster linker)" || echo "  $(YELLOW)✗ mold$(NC) - install: apt install mold (Linux only)"
	@command -v sccache >/dev/null 2>&1 && echo "  $(GREEN)✓ sccache$(NC) (compilation cache)" || echo "  $(YELLOW)✗ sccache$(NC) - install: brew install sccache (or: cargo install sccache --locked)"
	@cargo nextest --version >/dev/null 2>&1 && echo "  $(GREEN)✓ cargo-nextest$(NC) (faster tests)" || echo "  $(YELLOW)✗ cargo-nextest$(NC) - install: cargo install cargo-nextest --locked"
	@command -v cargo-watch >/dev/null 2>&1 && echo "  $(GREEN)✓ cargo-watch$(NC)" || echo "  $(YELLOW)✗ cargo-watch$(NC) - install: cargo install cargo-watch --locked"
	@command -v pre-commit >/dev/null 2>&1 && echo "  $(GREEN)✓ pre-commit$(NC)" || echo "  $(YELLOW)✗ pre-commit$(NC) - install: brew install pre-commit"

prod-up: ## Start the production stack
	docker compose --profile prod up

prod-up-d: ## Start the production stack (detached)
	docker compose --profile prod up -d

prod-down: ## Stop the production stack
	docker compose --profile prod down

prod-logs: ## Tail production logs
	docker compose --profile prod logs -f

# ── Docker image ─────────────────────────────────────────────────────────────

docker-build: ## Build the production Docker image
	docker build -t tsundoku:latest .

docker-build-clean-cache: ## Clear Docker buildx cache (use when build layers go stale)
	docker buildx prune --filter type=exec.cachemount -f

docker-run: ## Run the production image
	docker run -p 8080:8080 tsundoku:latest

docker-push: ## Push to a registry (usage: make docker-push REGISTRY=ghcr.io/you)
	@if [ -z "$(REGISTRY)" ]; then \
		echo "$(YELLOW)Error: REGISTRY not set. Use: make docker-push REGISTRY=ghcr.io/you$(NC)"; \
		exit 1; \
	fi
	docker tag tsundoku:latest $(REGISTRY)/tsundoku:latest
	docker push $(REGISTRY)/tsundoku:latest

# ── Changelog (git-cliff) ────────────────────────────────────────────────────

changelog: ## Generate the changelog and prepend to CHANGELOG.md
	git-cliff --unreleased --prepend CHANGELOG.md

changelog-unreleased: ## Preview unreleased changes
	git-cliff --unreleased

changelog-release: ## Generate the changelog for a version (usage: make changelog-release VERSION=1.0.0)
	@if [ -z "$(VERSION)" ]; then \
		echo "$(YELLOW)Error: VERSION not set. Use: make changelog-release VERSION=1.0.0$(NC)"; \
		exit 1; \
	fi
	@touch CHANGELOG.md
	git-cliff --unreleased --tag v$(VERSION) --prepend CHANGELOG.md
	@echo "$(GREEN)Changelog generated for v$(VERSION)$(NC)"

# ── Release ──────────────────────────────────────────────────────────────────

release-prepare: ## Prepare a release (usage: make release-prepare VERSION=1.0.0)
	@if [ -z "$(VERSION)" ]; then \
		echo "$(YELLOW)Error: VERSION not set. Use: make release-prepare VERSION=1.0.0$(NC)"; \
		exit 1; \
	fi
	@echo "$(BLUE)Preparing release v$(VERSION)...$(NC)"
	@echo ""

	@# Update Cargo.toml version
	@echo "$(YELLOW)Updating Cargo.toml version...$(NC)";
	@sed -i.bak 's/^version = ".*"/version = "$(VERSION)"/' Cargo.toml && rm Cargo.toml.bak
	@echo "$(GREEN)✓$(NC) Cargo.toml version set to $(VERSION)"

	@# Update web/package.json version
	@echo "$(YELLOW)Updating web/package.json version...$(NC)";
	@cd web && npm version $(VERSION) --no-git-tag-version --allow-same-version >/dev/null 2>&1
	@echo "$(GREEN)✓$(NC) web/package.json version set to $(VERSION)"

	@# Update docs/package.json version
	@echo "$(YELLOW)Updating docs/package.json version...$(NC)";
	@cd docs && npm version $(VERSION) --no-git-tag-version --allow-same-version >/dev/null 2>&1
	@echo "$(GREEN)✓$(NC) docs/package.json version set to $(VERSION)"

	@# Update Cargo.lock
	@echo "$(YELLOW)Updating Cargo.lock...$(NC)";
	@cargo build --quiet 2>/dev/null || cargo build
	@echo "$(GREEN)✓$(NC) Updated Cargo.lock"

	@# Regenerate OpenAPI spec and TypeScript types
	@echo "$(YELLOW)Regenerating OpenAPI spec and TypeScript types...$(NC)";
	@$(MAKE) openapi-all
	@echo "$(GREEN)✓$(NC) Regenerated OpenAPI spec and TypeScript types"

	@# Generate changelog (skip if already modified)
	@echo "$(YELLOW)Generating CHANGELOG.md...$(NC)";
	@if git diff --quiet CHANGELOG.md 2>/dev/null && git diff --cached --quiet CHANGELOG.md 2>/dev/null; then \
		$(MAKE) changelog-release VERSION=$(VERSION); \
		echo "$(GREEN)✓$(NC) Generated CHANGELOG.md for v$(VERSION)"; \
	else \
		echo "$(YELLOW)⊘$(NC) Skipped CHANGELOG.md (already modified)"; \
		echo "   To regenerate: git checkout CHANGELOG.md && make changelog-release VERSION=$(VERSION)"; \
	fi
	@echo ""
	@echo "$(BLUE)═══════════════════════════════════════════════════════════════$(NC)"
	@echo "$(GREEN)Release v$(VERSION) prepared!$(NC)"
	@echo "$(BLUE)═══════════════════════════════════════════════════════════════$(NC)"
	@echo ""
	@echo "$(YELLOW)Next steps:$(NC)"
	@echo "  1. Review the changes:"
	@echo "     $(GREEN)git diff$(NC)"
	@echo ""
	@echo "  2. Commit the release:"
	@echo "     $(GREEN)git add -A && git commit -m \"chore(release): v$(VERSION)\"$(NC)"
	@echo ""
	@echo "  3. Create the tag:"
	@echo "     $(GREEN)git tag -a v$(VERSION) -m \"v$(VERSION)\"$(NC)"
	@echo ""
	@echo "  4. Push to remote:"
	@echo "     $(GREEN)git push && git push --tags$(NC)"
	@echo ""

# ── Binary distribution (cargo-dist) ─────────────────────────────────────────
# Note: dist builds embed the frontend, so run `make frontend-build` first
# (or wire it as a cargo-dist build hook).

dist-install: ## Install cargo-dist
	cargo install cargo-dist --locked

dist-plan: ## Show what cargo-dist will build
	cargo dist plan

dist-build: ## Build distributable artifacts for all configured targets
	cargo dist build

dist-build-local: ## Build for the current platform only
	cargo dist build --artifacts=local

# ── Cleanup ──────────────────────────────────────────────────────────────────

clean: ## Remove build artifacts
	cargo clean
	rm -rf web/dist web/node_modules
	rm -rf docs/build docs/.docusaurus docs/.cache-loader docs/node_modules docs/docs/api

clean-docker: ## Stop all compose profiles and remove volumes
	docker compose --profile dev --profile prod down -v

clean-all: clean clean-docker ## Clean build artifacts AND tear down Docker (volumes included)
	@echo "$(GREEN)All build and Docker state cleared.$(NC)"

setup-hooks: ## Install git pre-commit hooks (if scripts/ provides them)
	@if [ -f scripts/setup-hooks.sh ]; then bash scripts/setup-hooks.sh; else echo "No scripts/setup-hooks.sh"; fi
