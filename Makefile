.DEFAULT_GOAL := help

CARGO ?= cargo
CLI_PKG ?= command-schema-cli
CLI := $(CARGO) run -q -p $(CLI_PKG) --

# Common knobs (override at invocation time, e.g. `make extract-commands COMMANDS=git,cargo`)
OUTPUT ?= /tmp/command-schema-output
TARGET_OUTPUT ?= /tmp/command-schema-targeted
COMMANDS ?= awk,pip3,lsof,ansible,less,ps
COMMAND ?= mycli
INPUT ?= /tmp/help.txt
FORMAT ?= json
CONFIG ?= ci-config.yaml
MANIFEST ?= /tmp/command-schema-manifest.json
SCHEMA_DIR ?= schemas/database
BUNDLE ?= /tmp/command-schemas-bundle.json
DB ?= /tmp/command-schemas.db
PREFIX ?= cs_
REPORT_OUTPUT ?= /tmp/command-schema-extraction-report.json

.PHONY: help
help: ## Show available make targets
	@awk 'BEGIN {FS = ":.*##"; printf "\nTargets:\n"} /^[a-zA-Z0-9_.-]+:.*##/ {printf "  %-22s %s\n", $$1, $$2} END {printf "\n"}' $(MAKEFILE_LIST)

.PHONY: check
check: ## Run cargo check for workspace
	$(CARGO) check --workspace

.PHONY: build
build: ## Build workspace
	$(CARGO) build --workspace

.PHONY: build-release
build-release: ## Build workspace in release mode
	$(CARGO) build --workspace --release

.PHONY: test
test: ## Run all tests
	$(CARGO) test --workspace

.PHONY: test-lib
test-lib: ## Run library tests
	$(CARGO) test --workspace --lib

.PHONY: test-discovery
test-discovery: ## Run discovery crate tests
	$(CARGO) test -p command-schema-discovery

.PHONY: test-db
test-db: ## Run db crate tests
	$(CARGO) test -p command-schema-db

.PHONY: test-cli
test-cli: ## Run cli crate tests
	$(CARGO) test -p command-schema-cli

.PHONY: fmt
fmt: ## Format Rust code
	$(CARGO) fmt

.PHONY: fmt-check
fmt-check: ## Check formatting without writing
	$(CARGO) fmt -- --check

.PHONY: clippy
clippy: ## Run clippy on workspace (deny warnings)
	$(CARGO) clippy --workspace --all-targets -- -D warnings

.PHONY: extract-allowlist
extract-allowlist: ## Extract installed allowlist commands to OUTPUT
	mkdir -p "$(OUTPUT)"
	$(CLI) extract --allowlist --installed-only --output "$(OUTPUT)" --no-cache

.PHONY: extract-commands
extract-commands: ## Extract COMMANDS CSV to TARGET_OUTPUT
	mkdir -p "$(TARGET_OUTPUT)"
	$(CLI) extract --commands "$(COMMANDS)" --installed-only --output "$(TARGET_OUTPUT)" --no-cache

.PHONY: extract-scan
extract-scan: ## Extract by scanning PATH into OUTPUT
	mkdir -p "$(OUTPUT)"
	$(CLI) extract --scan-path --installed-only --output "$(OUTPUT)" --no-cache

.PHONY: extract-repo-allowlist
extract-repo-allowlist: ## Non-destructive: extract allowlist and merge results into SCHEMA_DIR
	mkdir -p "$(SCHEMA_DIR)"
	@stage_dir="$$(mktemp -d /tmp/command-schema-stage-XXXXXX)"; \
	echo "Staging extraction in $$stage_dir"; \
	$(CLI) extract --allowlist --installed-only --output "$$stage_dir" --no-cache; \
	find "$$stage_dir" -maxdepth 1 -type f -name '*.json' ! -name 'extraction-report.json' -exec cp -f {} "$(SCHEMA_DIR)/" \;; \
	cp -f "$$stage_dir/extraction-report.json" "$(REPORT_OUTPUT)"; \
	echo "Merged extracted schemas into $(SCHEMA_DIR) without deleting existing files."; \
	echo "Extraction report: $(REPORT_OUTPUT)"; \
	rm -rf "$$stage_dir"

.PHONY: extract-repo-commands
extract-repo-commands: ## Non-destructive: extract COMMANDS and merge results into SCHEMA_DIR
	mkdir -p "$(SCHEMA_DIR)"
	@stage_dir="$$(mktemp -d /tmp/command-schema-stage-XXXXXX)"; \
	echo "Staging extraction in $$stage_dir"; \
	$(CLI) extract --commands "$(COMMANDS)" --installed-only --output "$$stage_dir" --no-cache; \
	find "$$stage_dir" -maxdepth 1 -type f -name '*.json' ! -name 'extraction-report.json' -exec cp -f {} "$(SCHEMA_DIR)/" \;; \
	cp -f "$$stage_dir/extraction-report.json" "$(REPORT_OUTPUT)"; \
	echo "Merged extracted schemas into $(SCHEMA_DIR) without deleting existing files."; \
	echo "Extraction report: $(REPORT_OUTPUT)"; \
	rm -rf "$$stage_dir"

.PHONY: validate-output
validate-output: ## Validate schema files in OUTPUT
	$(CLI) validate "$(OUTPUT)"

.PHONY: validate-target
validate-target: ## Validate schema files in TARGET_OUTPUT
	$(CLI) validate "$(TARGET_OUTPUT)"

.PHONY: parse-file
parse-file: ## Parse help text from INPUT for COMMAND
	$(CLI) parse-file --command "$(COMMAND)" --input "$(INPUT)" --format "$(FORMAT)"

.PHONY: parse-stdin
parse-stdin: ## Parse help text from stdin for COMMAND (pipe content into this)
	$(CLI) parse-stdin --command "$(COMMAND)" --format "$(FORMAT)"

.PHONY: ci-extract
ci-extract: ## Run ci-extract using CONFIG/MANIFEST/OUTPUT
	mkdir -p "$(OUTPUT)"
	$(CLI) ci-extract --config "$(CONFIG)" --manifest "$(MANIFEST)" --output "$(OUTPUT)"

.PHONY: bundle
bundle: ## Bundle SCHEMA_DIR into BUNDLE
	$(CLI) bundle "$(SCHEMA_DIR)" --output "$(BUNDLE)"

.PHONY: migrate-up
migrate-up: ## Create SQLite tables in DB with PREFIX
	$(CLI) migrate up --db "$(DB)" --prefix "$(PREFIX)"

.PHONY: migrate-down
migrate-down: ## Drop SQLite tables in DB with PREFIX
	$(CLI) migrate down --db "$(DB)" --prefix "$(PREFIX)"

.PHONY: migrate-seed
migrate-seed: ## Seed DB from SCHEMA_DIR
	$(CLI) migrate seed --db "$(DB)" --prefix "$(PREFIX)" --source "$(SCHEMA_DIR)"

.PHONY: migrate-refresh
migrate-refresh: ## Recreate and reseed DB from SCHEMA_DIR
	$(CLI) migrate refresh --db "$(DB)" --prefix "$(PREFIX)" --source "$(SCHEMA_DIR)"

.PHONY: migrate-status
migrate-status: ## Show DB migration status
	$(CLI) migrate status --db "$(DB)" --prefix "$(PREFIX)"

.PHONY: smoke
smoke: ## Quick sanity flow: extract targeted set, then validate it
	$(MAKE) extract-commands
	$(MAKE) validate-target

.PHONY: clean
clean: ## Clean Cargo build artifacts
	$(CARGO) clean
