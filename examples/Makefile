# ACME Platform - Build & Deploy Automation
# Usage: make help

SHELL := /bin/bash
.DEFAULT_GOAL := help
.PHONY: help build test lint fmt clean docker deploy

# --- Variables ---
PROJECT     := acme-platform
VERSION     := $(shell git describe --tags --always --dirty 2>/dev/null || echo "dev")
GIT_SHA     := $(shell git rev-parse --short HEAD 2>/dev/null || echo "unknown")
BUILD_TIME  := $(shell date -u +"%Y-%m-%dT%H:%M:%SZ")
REGISTRY    := ghcr.io/acme
CARGO_FLAGS ?=
DOCKER_BUILDKIT := 1

# Rust toolchain
CARGO       := cargo
RUSTFMT     := rustfmt
CLIPPY      := cargo clippy

# Database
DB_HOST     ?= localhost
DB_PORT     ?= 5432
DB_NAME     ?= acme_dev
DB_USER     ?= acme
DATABASE_URL ?= postgres://$(DB_USER)@$(DB_HOST):$(DB_PORT)/$(DB_NAME)

# Kubernetes
KUBE_CONTEXT ?= acme-production
KUBE_NS      ?= ecommerce
HELM_RELEASE ?= acme-platform

# --- Help ---

help: ## Show this help message
	@echo "$(PROJECT) v$(VERSION) ($(GIT_SHA))"
	@echo ""
	@echo "Targets:"
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2}'

# --- Build ---

build: ## Build all crates in release mode
	$(CARGO) build --release $(CARGO_FLAGS)

build-dev: ## Build all crates in debug mode
	$(CARGO) build $(CARGO_FLAGS)

build-api: ## Build only the API server
	$(CARGO) build --release --bin acme-api

build-worker: ## Build only the background worker
	$(CARGO) build --release --bin acme-worker

# --- Testing ---

test: ## Run all tests
	$(CARGO) test --workspace $(CARGO_FLAGS)

test-unit: ## Run unit tests only (no integration tests)
	$(CARGO) test --workspace --lib $(CARGO_FLAGS)

test-integration: ## Run integration tests (requires Docker)
	@echo "Starting test dependencies..."
	docker compose -f docker-compose.test.yml up -d --wait
	$(CARGO) test --workspace --test '*' $(CARGO_FLAGS) -- --test-threads=4
	docker compose -f docker-compose.test.yml down -v

test-coverage: ## Generate test coverage report
	$(CARGO) llvm-cov --workspace --html --output-dir target/coverage
	@echo "Coverage report: target/coverage/html/index.html"

bench: ## Run benchmarks
	$(CARGO) bench --workspace $(CARGO_FLAGS) 2>&1 | tee target/bench-$(GIT_SHA).txt

# --- Code Quality ---

lint: ## Run clippy linter
	$(CLIPPY) --workspace --all-targets -- -D warnings

fmt: ## Format code
	$(CARGO) fmt --all

fmt-check: ## Check code formatting
	$(CARGO) fmt --all -- --check

audit: ## Audit dependencies for vulnerabilities
	cargo audit --deny warnings

deny: ## Check dependency licenses and advisories
	cargo deny check licenses advisories

check: fmt-check lint test ## Run all checks (CI pipeline)

# --- Database ---

db-create: ## Create development database
	createdb -h $(DB_HOST) -p $(DB_PORT) -U $(DB_USER) $(DB_NAME) 2>/dev/null || true

db-migrate: ## Run database migrations
	migrate -path migrations/ -database "$(DATABASE_URL)?sslmode=disable" up

db-rollback: ## Rollback last migration
	migrate -path migrations/ -database "$(DATABASE_URL)?sslmode=disable" down 1

db-reset: db-drop db-create db-migrate db-seed ## Reset database completely

db-drop: ## Drop development database
	dropdb -h $(DB_HOST) -p $(DB_PORT) -U $(DB_USER) --if-exists $(DB_NAME)

db-seed: ## Seed database with sample data
	$(CARGO) run --bin acme-cli -- db seed --count 1000

db-console: ## Open psql console
	psql "$(DATABASE_URL)"

# --- Docker ---

docker-build: docker-build-api docker-build-worker ## Build all Docker images

docker-build-api: ## Build API Docker image
	DOCKER_BUILDKIT=1 docker build \
		--target api \
		--build-arg GIT_SHA=$(GIT_SHA) \
		--build-arg BUILD_TIME=$(BUILD_TIME) \
		--tag $(REGISTRY)/acme-api:$(VERSION) \
		--tag $(REGISTRY)/acme-api:latest \
		--file Dockerfile \
		.

docker-build-worker: ## Build worker Docker image
	DOCKER_BUILDKIT=1 docker build \
		--target worker \
		--build-arg GIT_SHA=$(GIT_SHA) \
		--build-arg BUILD_TIME=$(BUILD_TIME) \
		--tag $(REGISTRY)/acme-worker:$(VERSION) \
		--tag $(REGISTRY)/acme-worker:latest \
		--file Dockerfile \
		.

docker-push: ## Push images to registry
	docker push $(REGISTRY)/acme-api:$(VERSION)
	docker push $(REGISTRY)/acme-api:latest
	docker push $(REGISTRY)/acme-worker:$(VERSION)
	docker push $(REGISTRY)/acme-worker:latest

# --- Local Development ---

dev: ## Start local development environment
	docker compose up -d postgres redis kafka
	@echo "Waiting for services..."
	@sleep 3
	$(CARGO) run --bin acme-api -- --config config/development.toml

dev-worker: ## Start local worker
	$(CARGO) run --bin acme-worker -- --config config/development.toml --concurrency 4

dev-down: ## Stop local development environment
	docker compose down -v

# --- Deployment ---

deploy: ## Deploy to production (requires confirmation)
	@echo "Deploying $(PROJECT) v$(VERSION) to $(KUBE_CONTEXT)/$(KUBE_NS)"
	@read -p "Continue? [y/N] " confirm && [ "$$confirm" = "y" ] || exit 1
	helm upgrade $(HELM_RELEASE) deploy/helm/ \
		--kube-context $(KUBE_CONTEXT) \
		--namespace $(KUBE_NS) \
		--set image.tag=$(VERSION) \
		--set image.repository=$(REGISTRY) \
		--set git.sha=$(GIT_SHA) \
		--wait --timeout 10m

deploy-dry-run: ## Preview deployment changes
	helm diff upgrade $(HELM_RELEASE) deploy/helm/ \
		--kube-context $(KUBE_CONTEXT) \
		--namespace $(KUBE_NS) \
		--set image.tag=$(VERSION)

rollback: ## Rollback to previous deployment
	helm rollback $(HELM_RELEASE) \
		--kube-context $(KUBE_CONTEXT) \
		--namespace $(KUBE_NS) \
		--wait --timeout 5m

# --- Cleanup ---

clean: ## Remove build artifacts
	$(CARGO) clean
	rm -rf target/coverage target/doc

distclean: clean ## Remove all generated files
	rm -rf node_modules .cache
	docker compose down -v --rmi local 2>/dev/null || true
