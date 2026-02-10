# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-02-09

### Added

**Static Database Infrastructure:**
- Pre-extracted database of 107 command schemas in `schemas/database/`
- `SchemaDatabase` for O(1) in-memory lookups from directories or bundles
- `DatabaseBuilder` with fallback chain support across multiple sources
- `Manifest` tracking with version, fingerprint, and checksum-based change detection
- Optional compile-time schema bundling via `bundled-schemas` feature (zero I/O)

**SQLite Storage Backend:**
- Normalized 8-table schema for relational storage with full round-trip fidelity
- `Migration` lifecycle management (up/down/seed/refresh/status)
- `SchemaQuery` CRUD interface for runtime schema access
- Customizable table prefixes for multi-tenant databases
- Transaction-safe operations with cascading foreign key cleanup

**CI Automation:**
- GitHub Actions workflow for weekly schema extraction
- Version cross-referencing to minimize redundant re-extraction
- Manifest-based change detection (only commits changed schemas)
- Quality validation and reporting
- Comprehensive toolset installation (150+ commands)

**CLI Tool:**
- `schema-discover` binary for extraction and database management
- `parse-file` and `parse-stdin` commands for offline parsing
- `extract` command for batch extraction from installed commands
- `ci-extract` command for CI pipeline integration with manifest tracking
- JSON, YAML, and Markdown output formats

**Documentation:**
- Comprehensive rustdoc comments across all public APIs
- Working examples for all major use cases
- Integration guide for wrashpty and similar projects
- Performance benchmarks (startup <100ms, memory <10MB)

### Changed
- Refactored `command-schema-discovery` to library-only (no binary)
- Moved CLI functionality to dedicated `command-schema-cli` crate
- Updated workspace to Rust 2024 edition

### Performance
- Directory loading: ~100ms for 200 schemas
- O(1) lookups via HashMap: ~10M lookups/sec
- SQLite indexed queries by command name and source
- Bundled schemas: zero filesystem I/O

### Migration Guide

**Breaking changes:**
- CLI binary moved from `command-schema-discovery` to the new `command-schema-cli` crate.
  Consumers who previously ran the binary from `discovery` must now depend on `command-schema-cli`.
- `command-schema-discovery` is now library-only (no binary target).

**Integration notes:**
- For wrashpty integration: use `SchemaDatabase::builder()` with fallback chain
- For SQLite storage: use `Migration::new()` with custom prefix
- For CI automation: configure `ci-config.yaml` and use `schema-discover ci-extract`
