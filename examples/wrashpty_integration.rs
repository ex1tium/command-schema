//! Full integration example: two-tier architecture.
//!
//! Demonstrates the recommended integration pattern for applications that
//! need both fast O(1) lookups at runtime and persistent storage for learned
//! schemas. This is the architecture used by wrashpty and similar terminal
//! applications.
//!
//! # Architecture
//!
//! ```text
//!  ┌─────────────────────────┐
//!  │   In-memory HashMap     │  ← O(1) runtime lookups
//!  │   (SchemaDatabase)      │
//!  └──────────┬──────────────┘
//!             │ startup: load
//!  ┌──────────┴──────────────┐
//!  │   Static schemas (JSON) │  ← Pre-extracted, bundled
//!  └─────────────────────────┘
//!             +
//!  ┌─────────────────────────┐
//!  │   SQLite (learned)      │  ← Runtime persistence
//!  └─────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```bash
//! cargo run -p command-schema-examples --example wrashpty_integration
//! ```

use std::collections::HashMap;
use std::io::Write;

use command_schema_core::{
    CommandSchema, FlagSchema, SchemaSource, SubcommandSchema, ValueType,
};
use command_schema_db::SchemaDatabase;
use command_schema_sqlite::{Migration, SchemaQuery};
use rusqlite::Connection;

/// Application-level schema registry combining static and learned schemas.
struct SchemaRegistry {
    /// In-memory cache for O(1) lookups at runtime.
    cache: HashMap<String, CommandSchema>,
    /// SQLite connection for persisting learned schemas.
    query: SchemaQuery,
}

impl SchemaRegistry {
    /// Initializes the registry by loading static schemas and any
    /// previously learned schemas from SQLite.
    fn new(static_db: SchemaDatabase, query: SchemaQuery) -> Self {
        let mut cache = HashMap::new();

        // Load static schemas into the cache
        for name in static_db.commands() {
            if let Some(schema) = static_db.get(name) {
                cache.insert(name.to_string(), schema.clone());
            }
        }

        // Load learned schemas from SQLite (these override static ones)
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

    /// Learn a new schema: write to SQLite and update the cache.
    fn learn(&mut self, schema: CommandSchema) {
        let name = schema.command.clone();

        // Persist to SQLite
        if self.query.get_schema(&name).unwrap().is_some() {
            self.query.update_schema(&schema).unwrap();
        } else {
            self.query.insert_schema(&schema).unwrap();
        }

        // Update in-memory cache
        self.cache.insert(name, schema);
    }

    /// Returns the number of schemas in the cache.
    fn len(&self) -> usize {
        self.cache.len()
    }
}

fn main() {
    // === Phase 1: Prepare static schemas ===
    println!("=== Phase 1: Setup ===");

    let schema_dir = std::env::temp_dir().join("cs_wrashpty_example");
    std::fs::create_dir_all(&schema_dir).unwrap();

    // Write static schemas to a temporary directory
    let git = create_git_schema();
    let docker = create_docker_schema();
    write_schema_file(&schema_dir, &git);
    write_schema_file(&schema_dir, &docker);

    // Load static schemas
    let static_db = SchemaDatabase::from_dir(&schema_dir).unwrap();
    println!("Static schemas loaded: {}", static_db.len());

    // === Phase 2: Initialize SQLite ===
    let conn = Connection::open_in_memory().unwrap();
    let mut migration = Migration::new(conn, "cs_").unwrap();
    migration.up().unwrap();

    // Optionally seed SQLite with static schemas too (for querying)
    migration.seed(&schema_dir).unwrap();

    let conn = migration.into_connection();
    let query = SchemaQuery::new(conn, "cs_").unwrap();

    // === Phase 3: Create the registry ===
    println!("\n=== Phase 2: Registry initialization ===");
    let mut registry = SchemaRegistry::new(static_db, query);
    println!("Registry initialized with {} schemas", registry.len());

    // === Phase 4: Runtime lookups (O(1)) ===
    println!("\n=== Phase 3: Runtime lookups ===");
    let start = std::time::Instant::now();
    for _ in 0..10_000 {
        let _ = registry.get("git");
        let _ = registry.get("docker");
        let _ = registry.get("nonexistent");
    }
    let elapsed = start.elapsed();
    println!("30,000 lookups in {elapsed:.2?} ({:.0} lookups/sec)",
        30_000.0 / elapsed.as_secs_f64());

    if let Some(git) = registry.get("git") {
        println!("\ngit: {} flags, {} subcommands",
            git.global_flags.len(), git.subcommands.len());
    }

    // === Phase 5: Learn new schemas ===
    println!("\n=== Phase 4: Learning ===");

    // Simulate discovering a new command at runtime
    let mut ripgrep = CommandSchema::new("rg", SchemaSource::Learned);
    ripgrep.description = Some("Recursively search for a pattern".into());
    ripgrep.global_flags.push(
        FlagSchema::boolean(Some("-i"), Some("--ignore-case"))
            .with_description("Case insensitive search"),
    );
    ripgrep.global_flags.push(
        FlagSchema::with_value(Some("-t"), Some("--type"), ValueType::String)
            .with_description("Only search files matching TYPE"),
    );

    registry.learn(ripgrep);
    println!("Learned 'rg' schema");
    println!("Registry now has {} schemas", registry.len());

    // The learned schema is immediately available for lookups
    if let Some(rg) = registry.get("rg") {
        println!("rg: {} flags", rg.global_flags.len());
    }

    // Cleanup
    std::fs::remove_dir_all(&schema_dir).ok();
    println!("\nDone!");
}

fn write_schema_file(dir: &std::path::Path, schema: &CommandSchema) {
    let path = dir.join(format!("{}.json", schema.command));
    let mut f = std::fs::File::create(path).unwrap();
    serde_json::to_writer_pretty(&mut f, schema).unwrap();
    f.flush().unwrap();
}

fn create_git_schema() -> CommandSchema {
    let mut schema = CommandSchema::new("git", SchemaSource::Bootstrap);
    schema.description = Some("The stupid content tracker".into());
    schema.global_flags.push(
        FlagSchema::boolean(Some("-v"), Some("--verbose")),
    );
    schema.subcommands.push(
        SubcommandSchema::new("commit")
            .with_flag(FlagSchema::with_value(Some("-m"), Some("--message"), ValueType::String)),
    );
    schema.subcommands.push(SubcommandSchema::new("push"));
    schema
}

fn create_docker_schema() -> CommandSchema {
    let mut schema = CommandSchema::new("docker", SchemaSource::Bootstrap);
    schema.description = Some("A self-sufficient runtime for containers".into());
    schema.subcommands.push(SubcommandSchema::new("run"));
    schema.subcommands.push(SubcommandSchema::new("build"));
    schema
}
