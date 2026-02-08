# Integration Guide

This guide covers the different ways to consume command schemas in your application, from simple development setups to production-grade architectures.

## Overview

The `command-schema` library supports five consumption patterns, each suited to different use cases:

| Pattern | Crate | Best for |
|---------|-------|----------|
| [Directory loading](#pattern-1-directory-loading) | `command-schema-db` | Development, testing, CI |
| [Bundle loading](#pattern-2-bundle-loading) | `command-schema-db` | Single-file distribution |
| [Binary bundling](#pattern-3-binary-bundling) | `command-schema-db` | Zero-I/O embedded apps |
| [SQLite storage](#pattern-4-sqlite-storage) | `command-schema-sqlite` | Runtime persistence |
| [Hybrid (recommended)](#pattern-5-hybrid-recommended) | Both | Production applications |

## Pattern 1: Directory loading

**Use case**: Development, testing, CI pipelines.

Load schemas from a directory of individual JSON files. Each file contains a single `CommandSchema` serialized as JSON.

```rust,no_run
use command_schema_db::SchemaDatabase;

let db = SchemaDatabase::from_dir("schemas/database/").unwrap();

// O(1) lookup by command name
if let Some(schema) = db.get("git") {
    println!("git: {} flags, {} subcommands",
        schema.global_flags.len(), schema.subcommands.len());
}

// Iterate over all commands
for name in db.commands() {
    println!("  {name}");
}
```

**Performance**: ~100ms startup for ~200 schemas. Lookups are O(1) after loading.

**Pros**:
- Simple to set up and debug (human-readable JSON files)
- Easy to add, remove, or update individual schemas
- Works well with version control

**Cons**:
- Requires filesystem access at startup
- Slower startup than bundled approaches
- Directory must be distributed alongside the binary

## Pattern 2: Bundle loading

**Use case**: Single-file distribution, packaging.

Load schemas from a single `SchemaPackage` JSON file containing all schemas.

```rust,no_run
use command_schema_db::SchemaDatabase;

let db = SchemaDatabase::from_bundle("schemas.json").unwrap();
println!("Loaded {} schemas", db.len());
```

**Creating bundles**: Use the extraction CLI to produce a bundle:

```bash
schema-discover extract --installed-only --format bundle > schemas.json
```

Or construct one programmatically:

```rust
use command_schema_core::*;

let mut package = SchemaPackage::new("1.0.0", "2024-01-15T00:00:00Z");
package.schemas.push(CommandSchema::new("git", SchemaSource::Bootstrap));
package.schemas.push(CommandSchema::new("docker", SchemaSource::HelpCommand));

let json = serde_json::to_string_pretty(&package).unwrap();
```

**Pros**:
- Single file to distribute
- Slightly faster than directory loading (one file open)

**Cons**:
- Less granular than directory loading
- Still requires filesystem access

## Pattern 3: Binary bundling

**Use case**: Zero-I/O embedded applications, static linking.

Schemas are compressed at build time and embedded directly into the binary. At runtime, `SchemaDatabase::bundled()` decompresses and parses them with zero filesystem I/O.

### Build-time setup

1. Place JSON schema files in `schemas/database/`
2. Enable the `bundled-schemas` feature:

```toml
[dependencies]
command-schema-db = { version = "0.1", features = ["bundled-schemas"] }
```

3. The `build.rs` script compresses each schema and generates a `bundled.rs` file with embedded constants.

### Runtime usage

```rust,ignore
use command_schema_db::SchemaDatabase;

let db = SchemaDatabase::bundled().unwrap();
println!("Loaded {} schemas from binary", db.len());
```

### With fallback

```rust,no_run
use command_schema_db::SchemaDatabase;

let db = SchemaDatabase::builder()
    .from_dir("schemas/database/")    // Development: load from disk
    .with_bundled()                    // Production: use embedded
    .build()
    .unwrap();
```

**Pros**:
- Zero filesystem I/O at runtime
- Self-contained binary
- Fastest startup

**Cons**:
- Schemas are fixed at compile time
- Increases binary size
- Requires rebuild to update schemas

## Pattern 4: SQLite storage

**Use case**: Runtime persistence, learned schemas, queryable storage.

The SQLite backend stores schemas in normalized tables with full round-trip fidelity. All mutations use transactions for atomicity.

### Migration workflow

```rust,no_run
use command_schema_sqlite::Migration;
use rusqlite::Connection;

let conn = Connection::open("schemas.db").unwrap();
let mut migration = Migration::new(conn, "cs_").unwrap();

// Create tables (idempotent)
migration.up().unwrap();

// Seed from pre-extracted JSON files
let report = migration.seed("schemas/database/").unwrap();
println!("Seeded {} commands", report.commands_inserted);

// Check status
let status = migration.status().unwrap();
println!("Tables: {}, Commands: {}", status.tables_exist, status.command_count);

// Full reset (down + up + seed)
migration.refresh("schemas/database/").unwrap();
```

### Query API

```rust,no_run
use command_schema_sqlite::SchemaQuery;
use command_schema_core::{CommandSchema, SchemaSource};
use rusqlite::Connection;

let conn = Connection::open("schemas.db").unwrap();
let mut query = SchemaQuery::new(conn, "cs_").unwrap();

// CRUD operations
let schema = CommandSchema::new("mycli", SchemaSource::Learned);
query.insert_schema(&schema).unwrap();

let loaded = query.get_schema("mycli").unwrap();
assert!(loaded.is_some());

// Filter by source
let learned = query.get_by_source(SchemaSource::Learned).unwrap();

// Update and delete
query.update_schema(&schema).unwrap();
query.delete_schema("mycli").unwrap();
```

### Table prefix isolation

Multiple schema sets can coexist in the same database using different prefixes:

```rust,no_run
use command_schema_sqlite::Migration;
use rusqlite::Connection;

let conn = Connection::open("schemas.db").unwrap();

// Production schemas
let mut prod = Migration::new(Connection::open("schemas.db").unwrap(), "prod_").unwrap();
prod.up().unwrap();

// Test schemas
let mut test = Migration::new(Connection::open("schemas.db").unwrap(), "test_").unwrap();
test.up().unwrap();
```

**Pros**:
- Persistent storage survives restarts
- Queryable (filter by source, list all)
- Transaction-safe writes
- Supports runtime learning

**Cons**:
- Slower lookups than in-memory HashMap
- Requires SQLite dependency
- More complex setup than directory loading

## Pattern 5: Hybrid (recommended)

**Use case**: Production applications like terminal emulators that need both fast lookups and runtime learning.

This pattern combines an in-memory `HashMap` for O(1) lookups with SQLite for persistent storage. It provides the best of both worlds.

### Architecture

```text
┌─────────────────────────────┐
│   In-memory HashMap         │  ← O(1) runtime lookups
│   (SchemaDatabase cache)    │
└──────────┬──────────────────┘
           │ startup: load all
┌──────────┴──────────────────┐
│   Static schemas (JSON/     │  ← Pre-extracted, versioned
│   bundled/directory)        │
└─────────────────────────────┘
           +
┌─────────────────────────────┐
│   SQLite (learned schemas)  │  ← Runtime persistence
└─────────────────────────────┘
```

### Implementation

```rust,no_run
use std::collections::HashMap;
use command_schema_core::{CommandSchema, SchemaSource};
use command_schema_db::SchemaDatabase;
use command_schema_sqlite::{Migration, SchemaQuery};
use rusqlite::Connection;

struct SchemaRegistry {
    cache: HashMap<String, CommandSchema>,
    query: SchemaQuery,
}

impl SchemaRegistry {
    fn init(schema_dir: &str, db_path: &str) -> Self {
        // Phase 1: Load static schemas
        let static_db = SchemaDatabase::from_dir(schema_dir).unwrap();

        // Phase 2: Set up SQLite
        let conn = Connection::open(db_path).unwrap();
        let mut migration = Migration::new(conn, "cs_").unwrap();
        migration.up().unwrap();
        let conn = migration.into_connection();
        let query = SchemaQuery::new(conn, "cs_").unwrap();

        // Phase 3: Build in-memory cache
        let mut cache = HashMap::new();
        for name in static_db.commands() {
            if let Some(schema) = static_db.get(name) {
                cache.insert(name.to_string(), schema.clone());
            }
        }

        // Phase 4: Overlay learned schemas
        if let Ok(learned) = query.get_by_source(SchemaSource::Learned) {
            for schema in learned {
                cache.insert(schema.command.clone(), schema);
            }
        }

        Self { cache, query }
    }

    /// O(1) lookup for runtime use.
    fn get(&self, command: &str) -> Option<&CommandSchema> {
        self.cache.get(command)
    }

    /// Learn a new schema: persist to SQLite and update cache.
    fn learn(&mut self, schema: CommandSchema) {
        let name = schema.command.clone();
        if self.query.get_schema(&name).unwrap().is_some() {
            self.query.update_schema(&schema).unwrap();
        } else {
            self.query.insert_schema(&schema).unwrap();
        }
        self.cache.insert(name, schema);
    }
}
```

**Performance**:
- Startup: ~100ms (load static schemas + SQLite learned schemas)
- Lookups: O(1) via HashMap (~10M lookups/sec)
- Learning: ~1ms per schema (SQLite write + cache update)

**Pros**:
- Fastest possible lookups
- Learned schemas persist across restarts
- Static schemas versioned separately from learned ones
- Graceful degradation if SQLite is unavailable

**Cons**:
- More code to maintain
- Two data sources to keep in sync
- Higher memory usage (all schemas in HashMap)

## Best practices

### When to re-extract schemas

- **Version change**: The command reports a different `--version` string
- **Executable change**: For version-less commands, the executable's path, mtime, or size changed
- **Policy change**: Quality thresholds were adjusted
- **Checksum mismatch**: The on-disk schema JSON was manually edited

Use `Manifest::diff()` to detect these changes automatically:

```rust,no_run
use command_schema_db::{Manifest, QualityPolicyFingerprint};

let old = Manifest::load("manifest.json").unwrap();
let new = Manifest::load("manifest-new.json").unwrap();
let changed = old.diff(&new);
println!("Commands to re-extract: {:?}", changed);
```

### Quality policy configuration

```rust
use command_schema_discovery::extractor::ExtractionQualityPolicy;

// Production: strict thresholds
let production = ExtractionQualityPolicy {
    min_confidence: 0.7,
    min_coverage: 0.3,
    allow_low_quality: false,
};

// Development: accept everything
let development = ExtractionQualityPolicy::permissive();
```

### Error handling

All fallible operations return `Result` types. Use the `?` operator for propagation:

```rust,no_run
use command_schema_db::SchemaDatabase;

fn load_schemas() -> command_schema_db::Result<SchemaDatabase> {
    let db = SchemaDatabase::builder()
        .from_dir("schemas/database/")
        .from_bundle("schemas.json")
        .build()?;
    Ok(db)
}
```

### Testing strategies

1. **Unit tests**: Create schemas in-memory using builder methods
2. **Integration tests**: Use `Connection::open_in_memory()` for SQLite tests
3. **Snapshot tests**: Serialize schemas to JSON and compare against fixtures
4. **Round-trip tests**: Insert into SQLite and verify the loaded schema matches

```rust
use command_schema_core::*;
use command_schema_sqlite::{Migration, SchemaQuery};
use rusqlite::Connection;

fn round_trip_test(schema: &CommandSchema) {
    let conn = Connection::open_in_memory().unwrap();
    let mut migration = Migration::new(conn, "test_").unwrap();
    migration.up().unwrap();
    let conn = migration.into_connection();
    let mut query = SchemaQuery::new(conn, "test_").unwrap();

    query.insert_schema(schema).unwrap();
    let loaded = query.get_schema(&schema.command).unwrap().unwrap();
    assert_eq!(loaded.command, schema.command);
    assert_eq!(loaded.global_flags.len(), schema.global_flags.len());
}
```

## Migration guide

### From JSON-only to SQLite

1. Add `command-schema-sqlite` to your dependencies
2. Create the SQLite database and run migrations
3. Seed from your existing JSON directory
4. Update your lookup code to use `SchemaQuery` or the hybrid pattern

### From runtime probing to static database

1. Run the extraction CLI to produce JSON schemas:
   ```bash
   schema-discover extract --installed-only --jobs 4 --output-dir schemas/database/
   ```
2. Add `command-schema-db` to your dependencies
3. Load schemas with `SchemaDatabase::from_dir()`
4. Replace runtime probing with static lookups

### Versioning and compatibility

- The `SCHEMA_CONTRACT_VERSION` constant tracks the schema format version
- `SchemaPackage` includes a `schema_version` field for compatibility checking
- `Manifest` tracks the tool version used for extraction, enabling re-extraction when the tool is updated
