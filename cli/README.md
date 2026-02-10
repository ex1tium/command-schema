# command-schema-cli

Command-line interface for schema extraction and database management.

## Binary

`schema-discover` - Offline command schema discovery, extraction, bundling, and database management.

## Commands

### extract

Extract command schemas from local tool help output.

```sh
schema-discover extract --commands git,docker,cargo --output ./schemas
schema-discover extract --allowlist --output ./schemas --min-confidence 0.7
schema-discover extract --scan-path --output ./schemas --installed-only --jobs 4
```

### validate

Validate one or more schema JSON files.

```sh
schema-discover validate ./schemas/
schema-discover validate ./schemas/git.json ./schemas/docker.json
```

### bundle

Bundle schema JSON files into a SchemaPackage file.

```sh
schema-discover bundle ./schemas/ --output bundle.json --name "my-schemas"
```

### parse-stdin

Parse help text from stdin without executing commands.

```sh
git --help | schema-discover parse-stdin --command git
git --help | schema-discover parse-stdin --command git --with-report --format yaml
```

### parse-file

Parse help text from a file without executing commands.

```sh
schema-discover parse-file --command git --input git-help.txt
schema-discover parse-file --command git --input git-help.txt --with-report
```

### ci-extract

CI-optimized extraction with manifest-based version tracking and parallel extraction.
Only re-extracts commands whose version, executable fingerprint, or quality policy has changed.

```sh
# First run creates the manifest
schema-discover ci-extract \
  --config ci-config.yaml \
  --manifest manifest.json \
  --output ./schemas

# Subsequent runs skip unchanged commands
schema-discover ci-extract \
  --config ci-config.yaml \
  --manifest manifest.json \
  --output ./schemas

# Force re-extraction of all commands
schema-discover ci-extract \
  --config ci-config.yaml \
  --manifest manifest.json \
  --output ./schemas \
  --force
```

### migrate

SQLite database migration and seeding operations.

```sh
# Create tables
schema-discover migrate up --db schemas.db --prefix cs_

# Seed from extracted schemas
schema-discover migrate seed --db schemas.db --prefix cs_ --source ./schemas

# Check status
schema-discover migrate status --db schemas.db --prefix cs_

# Drop and recreate with fresh data
schema-discover migrate refresh --db schemas.db --prefix cs_ --source ./schemas

# Drop all tables
schema-discover migrate down --db schemas.db --prefix cs_
```

## CI Workflow Example

```sh
# 1. Extract schemas with version tracking
schema-discover ci-extract \
  --config ci-config.yaml \
  --manifest manifest.json \
  --output ./schemas

# 2. Create/update SQLite database
schema-discover migrate refresh \
  --db schemas.db \
  --prefix cs_ \
  --source ./schemas

# 3. Bundle for distribution
schema-discover bundle ./schemas/ --output schemas-bundle.json
```

## Local Development

```sh
# Extract a few commands for testing
schema-discover extract --commands git,cargo --output ./dev-schemas

# Parse help text from a file
schema-discover parse-file --command mycli --input help-output.txt --with-report

# Set up a local database
schema-discover migrate up --db local.db --prefix dev_
schema-discover migrate seed --db local.db --prefix dev_ --source ./dev-schemas
schema-discover migrate status --db local.db --prefix dev_
```
