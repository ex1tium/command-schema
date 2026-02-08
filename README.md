# command-schema

Extract structured schemas from CLI `--help` output.

Given a command's help text, `command-schema` parses it into a typed schema describing flags, positional arguments, subcommands, and their relationships. The library supports multiple storage backends for schema persistence and distribution.

## Crates

| Crate | Description |
|-------|-------------|
| [`command-schema-core`](core/) | Core types: `CommandSchema`, `FlagSchema`, `ArgSchema`, `SubcommandSchema`, validation, merging |
| [`command-schema-discovery`](discovery/) | Help text parser, command probing, extraction CLI (`schema-discover`), caching, reporting |
| [`command-schema-db`](db/) | Static database loading from directories/bundles, manifest tracking, CI configuration, optional compile-time bundling |
| [`command-schema-sqlite`](sqlite/) | SQLite storage backend with normalized tables, migration lifecycle, and CRUD query interface |

## Quick start

### Parsing help text

```rust
use command_schema_discovery::parse_help_text;

let result = parse_help_text("ls", include_str!("help-output.txt"));
if let Some(schema) = result.schema {
    for flag in &schema.global_flags {
        println!("{:?} — {:?}", flag.long, flag.description);
    }
}
```

### Loading a static database

```rust,no_run
use command_schema_db::SchemaDatabase;

// Load from a directory of JSON schema files
let db = SchemaDatabase::from_dir("schemas/database/").unwrap();
if let Some(schema) = db.get("git") {
    println!("git has {} subcommands", schema.subcommands.len());
}

// Or use a builder with fallback chain
let db = SchemaDatabase::builder()
    .from_dir("schemas/database/")    // Try directory first
    .from_bundle("schemas.json")      // Fall back to bundle
    .with_bundled()                    // Fall back to compiled-in
    .build()
    .unwrap();
```

### SQLite storage

```rust,no_run
use command_schema_sqlite::{Migration, SchemaQuery};
use command_schema_core::{CommandSchema, SchemaSource};
use rusqlite::Connection;

// Set up the database
let conn = Connection::open("schemas.db").unwrap();
let mut migration = Migration::new(conn, "cs_").unwrap();
migration.up().unwrap();
migration.seed("schemas/database/").unwrap();

// Query schemas
let conn = migration.into_connection();
let mut query = SchemaQuery::new(conn, "cs_").unwrap();

if let Some(schema) = query.get_schema("git").unwrap() {
    println!("{} has {} subcommands", schema.command, schema.subcommands.len());
}

// Insert a learned schema at runtime
let schema = CommandSchema::new("mycli", SchemaSource::Learned);
query.insert_schema(&schema).unwrap();
```

### As a CLI

```bash
# Parse help from a file
schema-discover parse-file --command ls --path ls-help.txt --format json

# Parse help from stdin
curl --help | schema-discover parse-stdin --command curl

# Extract schemas from installed commands
schema-discover extract --installed-only --jobs 4
```

## Features

### Supported help formats

The parser detects and handles multiple help output conventions:

- GNU (`--long-flag`, `-s` short flags)
- Clap (Rust) / Cobra (Go) / Argparse (Python) style
- NPM-style subcommand listings
- BSD-style flags
- Generic section-based help

### Static database

Pre-extracted schemas are stored as individual JSON files and loaded into an in-memory `HashMap` for O(1) lookups. The `DatabaseBuilder` supports fallback chains across multiple sources.

### SQLite support

The SQLite backend provides normalized storage with 8 tables covering commands, subcommands, flags, positional args, choices, aliases, and flag relationships. All mutations use transactions for atomicity, and cascading foreign keys handle cleanup automatically.

### Multiple consumption patterns

| Pattern | Use case | Startup | I/O |
|---------|----------|---------|-----|
| Directory loading | Development, testing, CI | ~100ms | Filesystem |
| Bundle loading | Single-file distribution | ~50ms | Single file |
| Binary bundling | Zero-I/O embedded apps | ~10ms | None |
| SQLite | Runtime persistence, learned schemas | ~20ms | Database |

## Examples

Working examples are available in each crate's `examples/` directory:

| Example | Crate | Description |
|---------|-------|-------------|
| [`parse_help`](discovery/examples/parse_help.rs) | discovery | Basic help text parsing |
| [`load_static_db`](db/examples/load_static_db.rs) | db | Directory loading and O(1) lookups |
| [`bundled_schemas`](db/examples/bundled_schemas.rs) | db | Compile-time embedded schemas |
| [`sqlite_migration`](sqlite/examples/sqlite_migration.rs) | sqlite | Complete SQLite lifecycle |
| [`wrashpty_integration`](sqlite/examples/wrashpty_integration.rs) | sqlite | Two-tier architecture (HashMap + SQLite) |

Run an example:

```bash
cargo run -p command-schema-discovery --example parse_help
cargo run -p command-schema-db --example load_static_db
cargo run -p command-schema-sqlite --example sqlite_migration
```

## Performance

- **Startup**: Loading ~200 schemas from a directory takes under 100ms
- **Lookups**: O(1) via in-memory `HashMap` (~10M lookups/sec)
- **SQLite**: Transaction-safe writes, indexed queries by command name and source
- **Bundled**: Zero filesystem I/O when schemas are compiled into the binary

## License

MIT
