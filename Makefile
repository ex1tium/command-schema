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
SCHEMA_BRANCH ?= schemas
SCHEMA_REMOTE ?= origin
SCHEMA_COMMIT_MSG ?= Update command schemas
SCHEMA_PUSH ?= 1
BUNDLE ?= /tmp/command-schemas-bundle.json
DB ?= /tmp/command-schemas.db
PREFIX ?= cs_
REPORT_OUTPUT ?= /tmp/command-schema-extraction-report.json
LIST_GLOB ?= schemas/command-lists/*.csv
FORCE ?= 0
JOBS ?= 8

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
	$(CLI) extract --allowlist --installed-only --jobs "$(JOBS)" --output "$(OUTPUT)" --no-cache

.PHONY: extract-commands
extract-commands: ## Extract COMMANDS CSV to TARGET_OUTPUT
	mkdir -p "$(TARGET_OUTPUT)"
	$(CLI) extract --commands "$(COMMANDS)" --installed-only --jobs "$(JOBS)" --output "$(TARGET_OUTPUT)" --no-cache

.PHONY: extract-scan
extract-scan: ## Extract by scanning PATH into OUTPUT
	mkdir -p "$(OUTPUT)"
	$(CLI) extract --scan-path --installed-only --jobs "$(JOBS)" --output "$(OUTPUT)" --no-cache

.PHONY: extract-repo-allowlist
extract-repo-allowlist: ## Non-destructive: extract allowlist and merge results into SCHEMA_DIR
	mkdir -p "$(SCHEMA_DIR)"
	@stage_dir="$$(mktemp -d /tmp/command-schema-stage-XXXXXX)"; \
	echo "Staging extraction in $$stage_dir"; \
	$(CLI) extract --allowlist --installed-only --jobs "$(JOBS)" --output "$$stage_dir" --no-cache; \
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
	$(CLI) extract --commands "$(COMMANDS)" --installed-only --jobs "$(JOBS)" --output "$$stage_dir" --no-cache; \
	find "$$stage_dir" -maxdepth 1 -type f -name '*.json' ! -name 'extraction-report.json' -exec cp -f {} "$(SCHEMA_DIR)/" \;; \
	cp -f "$$stage_dir/extraction-report.json" "$(REPORT_OUTPUT)"; \
	echo "Merged extracted schemas into $(SCHEMA_DIR) without deleting existing files."; \
	echo "Extraction report: $(REPORT_OUTPUT)"; \
	rm -rf "$$stage_dir"

.PHONY: extract-repo-system
extract-repo-system: ## Scan PATH + LIST_GLOB CSV lists, extract installed commands, merge into SCHEMA_DIR (FORCE=1 overwrites)
	mkdir -p "$(SCHEMA_DIR)"
	@set -eu; \
	stage_dir="$$(mktemp -d /tmp/command-schema-stage-XXXXXX)"; \
	list_tmp="$$(mktemp /tmp/command-schema-list-XXXXXX)"; \
	cleanup() { rm -rf "$$stage_dir" "$$list_tmp"; }; \
	trap cleanup EXIT INT TERM; \
	echo "Staging extraction in $$stage_dir"; \
	echo "Collecting commands from $(LIST_GLOB)"; \
	cat $(LIST_GLOB) 2>/dev/null | tr ',\r' '\n' | sed 's/^[[:space:]]*//;s/[[:space:]]*$$//' | grep -E '^[A-Za-z0-9][A-Za-z0-9+._-]*$$' | sort -u > "$$list_tmp" || true; \
	if [ -s "$$list_tmp" ]; then \
		commands_csv="$$(paste -sd, "$$list_tmp")"; \
		echo "List-source commands: $$(wc -l < "$$list_tmp")"; \
		$(CLI) extract --scan-path --commands "$$commands_csv" --installed-only --jobs "$(JOBS)" --output "$$stage_dir" --no-cache; \
	else \
		echo "No valid commands found in list files; running scan-path only."; \
		$(CLI) extract --scan-path --installed-only --jobs "$(JOBS)" --output "$$stage_dir" --no-cache; \
	fi; \
	if [ ! -f "$$stage_dir/extraction-report.json" ]; then \
		echo "Extraction failed: missing extraction-report.json in $$stage_dir"; \
		exit 1; \
	fi; \
	if [ "$(FORCE)" = "1" ] || [ "$(FORCE)" = "true" ] || [ "$(FORCE)" = "yes" ]; then \
		cp_mode="-f"; \
		echo "Merge mode: overwrite existing schemas (FORCE=$(FORCE))"; \
	else \
		cp_mode="-n"; \
		echo "Merge mode: keep existing schemas (FORCE=$(FORCE))"; \
	fi; \
	find "$$stage_dir" -maxdepth 1 -type f -name '*.json' ! -name 'extraction-report.json' -exec cp $$cp_mode {} "$(SCHEMA_DIR)/" \;; \
	cp -f "$$stage_dir/extraction-report.json" "$(REPORT_OUTPUT)"; \
	echo "Merged extracted schemas into $(SCHEMA_DIR). Existing files for missing commands were left untouched."; \
	echo "Extraction report: $(REPORT_OUTPUT)"

.PHONY: extract-repo-system-force
extract-repo-system-force: ## Same as extract-repo-system with overwrite enabled
	$(MAKE) extract-repo-system FORCE=1

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

.PHONY: fetch-schemas
fetch-schemas: ## Fetch pre-extracted schemas from the schemas branch into SCHEMA_DIR
	@echo "Fetching schemas from branch '$(SCHEMA_BRANCH)' into $(SCHEMA_DIR)/"
	@git fetch origin $(SCHEMA_BRANCH)
	@mkdir -p "$(SCHEMA_DIR)"
	@git archive origin/$(SCHEMA_BRANCH) | tar -x -C "$(SCHEMA_DIR)/"
	@echo "Fetched $$(ls "$(SCHEMA_DIR)"/*.json 2>/dev/null | wc -l) schema files."

.PHONY: publish-schemas
publish-schemas: ## Sync SCHEMA_DIR *.json into SCHEMA_BRANCH (create/update/delete), then commit and optionally push
	@set -eu; \
	src_dir="$(SCHEMA_DIR)"; \
	remote="$(SCHEMA_REMOTE)"; \
	branch="$(SCHEMA_BRANCH)"; \
	if [ ! -d "$$src_dir" ]; then \
		echo "SCHEMA_DIR does not exist: $$src_dir"; \
		exit 1; \
	fi; \
	worktree_dir="$$(mktemp -d /tmp/command-schema-publish-XXXXXX)"; \
	cleanup() { \
		git worktree remove --force "$$worktree_dir" >/dev/null 2>&1 || true; \
		rm -rf "$$worktree_dir"; \
	}; \
	trap cleanup EXIT INT TERM; \
	echo "Preparing branch '$$branch' from remote '$$remote' in $$worktree_dir"; \
	git fetch "$$remote" "$$branch" >/dev/null 2>&1 || true; \
	if git ls-remote --exit-code --heads "$$remote" "$$branch" >/dev/null 2>&1; then \
		git worktree add -B "$$branch" "$$worktree_dir" "$$remote/$$branch" >/dev/null; \
	else \
		echo "Remote branch '$$remote/$$branch' not found. Creating orphan branch '$$branch'."; \
		git worktree add --detach "$$worktree_dir" HEAD >/dev/null; \
		git -C "$$worktree_dir" checkout --orphan "$$branch" >/dev/null; \
		git -C "$$worktree_dir" rm -rf . >/dev/null 2>&1 || true; \
	fi; \
	find "$$worktree_dir" -maxdepth 1 -type f -name '*.json' ! -name 'manifest.json' -delete; \
	find "$$src_dir" -maxdepth 1 -type f -name '*.json' ! -name 'manifest.json' -exec cp -f {} "$$worktree_dir"/ \;; \
	count="$$(find "$$worktree_dir" -maxdepth 1 -type f -name '*.json' ! -name 'manifest.json' | wc -l)"; \
	echo "Staged $$count schema file(s) in branch workspace."; \
	git -C "$$worktree_dir" add -A; \
	if git -C "$$worktree_dir" diff --cached --quiet; then \
		echo "No schema changes detected; nothing to commit."; \
		exit 0; \
	fi; \
	git -C "$$worktree_dir" commit -m "$(SCHEMA_COMMIT_MSG)"; \
	if [ "$(SCHEMA_PUSH)" = "1" ] || [ "$(SCHEMA_PUSH)" = "true" ] || [ "$(SCHEMA_PUSH)" = "yes" ]; then \
		git -C "$$worktree_dir" push "$$remote" "$$branch"; \
		echo "Pushed to $$remote/$$branch"; \
	else \
		echo "SCHEMA_PUSH=$(SCHEMA_PUSH); commit created locally in worktree, push skipped."; \
	fi

.PHONY: clean-schemas
clean-schemas: ## Remove locally fetched schemas
	rm -rf "$(SCHEMA_DIR)"

.PHONY: clean
clean: ## Clean Cargo build artifacts
	$(CARGO) clean
