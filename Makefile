.PHONY: help \
	build build-release run watch \
	fmt lint check ci test test-fast \
	frontend frontend-build frontend-install frontend-lint frontend-lint-fix test-frontend \
	openapi openapi-types openapi-all \
	docs docs-build docs-install \
	dev-up dev-up-d dev-up-build dev-down dev-down-v dev-logs dev-watch dev-shell \
	prod-up prod-up-d prod-down prod-logs \
	docker-build docker-run docker-push \
	changelog changelog-unreleased changelog-release release-prepare \
	dist-install dist-plan dist-build dist-build-local \
	clean clean-docker setup-hooks

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

frontend-lint: ## Lint the frontend (biome)
	cd web && npm run lint

frontend-lint-fix: ## Lint and auto-fix the frontend
	cd web && npm run lint:fix

test-frontend: ## Run frontend tests
	cd web && npm run test:run

# ── Docs site (Docusaurus) ───────────────────────────────────────────────────

docs: ## Start the Docusaurus dev server at http://localhost:3000
	cd docs && npm install && npm run start

docs-build: ## Build the docs site into docs/build (Cloudflare Pages publishes this)
	cd docs && npm ci && npm run build

docs-install: ## Install docs site dependencies
	cd docs && npm install

# ── OpenAPI ──────────────────────────────────────────────────────────────────

openapi: ## Generate the OpenAPI spec from the backend
	cargo run -- openapi --output web/openapi.json
	@echo "$(GREEN)OpenAPI spec written to web/openapi.json$(NC)"

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

dev-logs: ## Tail dev stack logs
	docker compose --profile dev logs -f

dev-watch: ## Start the dev stack with docker compose watch (auto-sync)
	docker compose -f docker-compose.yml -f compose.watch.yml --profile dev watch

dev-shell: ## Open a shell in the dev backend container
	docker exec -it tsundoku-dev sh

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
	git-cliff --unreleased --tag v$(VERSION) --prepend CHANGELOG.md
	@echo "$(GREEN)Changelog generated for v$(VERSION)$(NC)"

# ── Release prep ─────────────────────────────────────────────────────────────

release-prepare: ## Bump versions + changelog + types (usage: make release-prepare VERSION=1.0.0)
	@if [ -z "$(VERSION)" ]; then \
		echo "$(YELLOW)Error: VERSION not set. Use: make release-prepare VERSION=1.0.0$(NC)"; \
		exit 1; \
	fi
	@echo "$(BLUE)Preparing release v$(VERSION)...$(NC)"
	@sed -i.bak 's/^version = ".*"/version = "$(VERSION)"/' Cargo.toml && rm Cargo.toml.bak
	@echo "$(GREEN)✓$(NC) Cargo.toml version set to $(VERSION)"
	@cd web && npm version $(VERSION) --no-git-tag-version --allow-same-version >/dev/null 2>&1
	@echo "$(GREEN)✓$(NC) web/package.json version set to $(VERSION)"
	@cargo build --quiet 2>/dev/null || cargo build
	@echo "$(GREEN)✓$(NC) Cargo.lock updated"
	@$(MAKE) openapi-all
	@echo "$(GREEN)✓$(NC) Regenerated OpenAPI spec and TS types"
	@$(MAKE) changelog-release VERSION=$(VERSION)
	@echo ""
	@echo "$(GREEN)Release v$(VERSION) prepared.$(NC) Next:"
	@echo "  git add -A && git commit -m \"chore(release): v$(VERSION)\""
	@echo "  git tag -a v$(VERSION) -m \"v$(VERSION)\""
	@echo "  git push && git push --tags"

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

clean-docker: ## Stop all compose profiles and remove volumes
	docker compose --profile dev --profile prod down -v

setup-hooks: ## Install git pre-commit hooks (if scripts/ provides them)
	@if [ -f scripts/setup-hooks.sh ]; then bash scripts/setup-hooks.sh; else echo "No scripts/setup-hooks.sh"; fi
