//! Full integration example: two-tier architecture with bundled schema support.
//!
//! Demonstrates the recommended integration pattern for applications that
//! need both fast O(1) lookups at runtime and persistent storage for learned
//! schemas. This is the architecture used by wrashpty and similar terminal
//! applications.
//!
//! # Architecture (Flow 4: Bundled → Directory → SQLite Fallback Chain)
//!
//! ```text
//!  ┌─────────────────────────┐
//!  │   In-memory HashMap     │  ← O(1) runtime lookups
//!  │   (SchemaDatabase)      │
//!  └──────────┬──────────────┘
//!             │ startup: load
//!  ┌──────────┴──────────────┐
//!  │  Bundled schemas (gzip) │  ← Tier 1: Zero-I/O, build-time embedded
//!  └─────────────────────────┘
//!             ↓ fallback
//!  ┌─────────────────────────┐
//!  │   Static schemas (JSON) │  ← Tier 2: Directory-based, file I/O
//!  └─────────────────────────┘
//!             +
//!  ┌─────────────────────────┐
//!  │   SQLite (learned)      │  ← Runtime persistence for discovered schemas
//!  └─────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```bash
//! # Without bundled schemas (directory fallback)
//! cargo run -p command-schema-examples --example wrashpty_integration
//!
//! # With bundled schemas (zero-I/O startup)
//! cargo run -p command-schema-examples --example wrashpty_integration --features bundled-schemas
//! ```

use std::collections::HashMap;
use std::io::Write;

use command_schema_core::{CommandSchema, FlagSchema, SchemaSource, SubcommandSchema, ValueType};
use command_schema_db::SchemaDatabase;
use command_schema_sqlite::{Migration, SchemaQuery};
use rusqlite::Connection;

/// Application-level schema registry combining static and learned schemas.
struct SchemaRegistry<'a> {
    /// In-memory cache for O(1) lookups at runtime.
    cache: HashMap<String, CommandSchema>,
    /// SQLite query interface for persisting learned schemas.
    query: SchemaQuery<'a>,
}

impl<'a> SchemaRegistry<'a> {
    /// Initializes the registry by loading static schemas and any
    /// previously learned schemas from SQLite.
    fn new(static_db: SchemaDatabase, query: SchemaQuery<'a>) -> Self {
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

        // Persist to SQLite (upsert: try insert, fall back to update on conflict)
        match self.query.get_schema(&name) {
            Ok(Some(_)) => self.query.update_schema(&schema).unwrap(),
            Ok(None) => self.query.insert_schema(&schema).unwrap(),
            Err(e) => panic!("Failed to check schema existence: {e}"),
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
    // === Phase 1: Load static schemas (Bundled → Directory fallback) ===
    println!("=== Phase 1: Static Schema Loading ===");

    let schema_dir = std::env::temp_dir().join("cs_wrashpty_example");
    std::fs::create_dir_all(&schema_dir).unwrap();

    // Write static schemas to a temporary directory (simulates pre-extracted schemas)
    let git = create_git_schema();
    let docker = create_docker_schema();
    write_schema_file(&schema_dir, &git);
    write_schema_file(&schema_dir, &docker);

    // Measure startup time for static schema loading
    let start = std::time::Instant::now();

    // Use the builder pattern with bundled → directory fallback chain.
    // When bundled-schemas feature is enabled, bundled schemas are tried first
    // (zero file I/O). If unavailable, falls back to directory loading.
    let static_db = SchemaDatabase::builder()
        .with_bundled() // Tier 1: Try bundled first (zero I/O)
        .from_dir(&schema_dir) // Tier 2: Fallback to directory
        .build()
        .expect("Failed to load static schemas");

    let startup_elapsed = start.elapsed();

    // Report which source was used
    #[cfg(feature = "bundled-schemas")]
    {
        println!("  Source: Bundled schemas (zero I/O, build-time embedded)");
    }
    #[cfg(not(feature = "bundled-schemas"))]
    {
        println!("  Source: Directory fallback (file I/O)");
    }

    println!("  Schemas loaded: {}", static_db.len());
    println!("  Startup time: {:.2?}", startup_elapsed);

    // Estimate memory usage by serializing the loaded schemas
    let memory_estimate = estimate_memory_usage(&static_db);
    println!("  Estimated memory: {:.3} MB", memory_estimate);

    // === Phase 2: Initialize SQLite ===
    let conn = Connection::open_in_memory().unwrap();
    let migration = Migration::new(&conn, "cs_").unwrap();
    migration.up().unwrap();

    // Optionally seed SQLite with static schemas too (for querying)
    migration.seed(&schema_dir).unwrap();
    drop(migration);

    let query = SchemaQuery::new(&conn, "cs_").unwrap();

    // === Phase 3: Create the registry (merge static + learned) ===
    println!("\n=== Phase 3: Registry Initialization ===");
    let mut registry = SchemaRegistry::new(static_db, query);
    println!("  Registry initialized with {} schemas", registry.len());

    // === Phase 4: Runtime lookups (O(1)) ===
    println!("\n=== Phase 4: Runtime Lookups ===");
    let start = std::time::Instant::now();
    for _ in 0..10_000 {
        let _ = registry.get("git");
        let _ = registry.get("docker");
        let _ = registry.get("nonexistent");
    }
    let elapsed = start.elapsed();
    println!(
        "  30,000 lookups in {elapsed:.2?} ({:.0} lookups/sec)",
        30_000.0 / elapsed.as_secs_f64()
    );

    if let Some(git) = registry.get("git") {
        println!(
            "  git: {} flags, {} subcommands",
            git.global_flags.len(),
            git.subcommands.len()
        );
    }

    // === Phase 5: Learn new schemas ===
    println!("\n=== Phase 5: Learning ===");

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
    println!("  Learned 'rg' schema");
    println!("  Registry now has {} schemas", registry.len());

    // The learned schema is immediately available for lookups
    if let Some(rg) = registry.get("rg") {
        println!("  rg: {} flags", rg.global_flags.len());
    }

    // === Summary ===
    println!("\n=== Performance Summary ===");
    println!("  Static schema startup: {:.2?}", startup_elapsed);
    println!("  Memory footprint: {:.3} MB", memory_estimate);
    println!("  Schema count: {}", registry.len());

    // Cleanup
    std::fs::remove_dir_all(&schema_dir).ok();
    println!("\nDone!");
}

/// Estimates the memory usage of a SchemaDatabase by serializing its contents.
fn estimate_memory_usage(db: &SchemaDatabase) -> f64 {
    let mut total_bytes = 0usize;
    for name in db.commands() {
        if let Some(schema) = db.get(name) {
            if let Ok(json) = serde_json::to_string(schema) {
                total_bytes += json.len();
            }
        }
    }
    total_bytes as f64 / (1024.0 * 1024.0)
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
    schema
        .global_flags
        .push(FlagSchema::boolean(Some("-v"), Some("--verbose")));
    schema.subcommands.push(
        SubcommandSchema::new("commit").with_flag(FlagSchema::with_value(
            Some("-m"),
            Some("--message"),
            ValueType::String,
        )),
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
