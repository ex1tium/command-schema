//! Bundled schemas example.
//!
//! Demonstrates how schemas can be embedded at compile time using the
//! `bundled-schemas` feature, providing zero-I/O startup.
//!
//! # Usage
//!
//! ```bash
//! # Without bundled schemas (demonstrates fallback)
//! cargo run -p command-schema-examples --example bundled_schemas
//!
//! # With bundled schemas (requires schemas in schemas/database/)
//! cargo run -p command-schema-examples --features bundled-schemas --example bundled_schemas
//! ```
//!
//! In production, `build.rs` compresses schema JSON files from
//! `schemas/database/` and embeds them as `BUNDLED_SCHEMAS` constants.
//! At runtime, `SchemaDatabase::bundled()` decompresses and parses them
//! with zero filesystem I/O.

use command_schema_db::SchemaDatabase;

fn main() {
    // Try loading bundled schemas (compiled into the binary)
    #[cfg(feature = "bundled-schemas")]
    {
        println!("=== Bundled schemas (zero I/O) ===");
        match SchemaDatabase::bundled() {
            Ok(db) => {
                println!("Loaded {} bundled schemas", db.len());
                for name in db.commands() {
                    println!("  {name}");
                }
            }
            Err(e) => {
                println!("No bundled schemas available: {e}");
                println!(
                    "(This is expected if no schemas were in schemas/database/ at build time)"
                );
            }
        }
    }

    #[cfg(not(feature = "bundled-schemas"))]
    {
        println!("=== Bundled schemas feature not enabled ===");
        println!(
            "Run with: cargo run -p command-schema-examples --features bundled-schemas --example bundled_schemas"
        );
        println!();
    }

    // Builder pattern with bundled schemas as fallback
    println!("=== Builder with fallback chain ===");
    let result = SchemaDatabase::builder()
        .from_dir("schemas/database/") // Try directory first (development)
        .with_bundled() // Fall back to bundled (production)
        .build();

    match result {
        Ok(db) => {
            println!("Loaded {} schemas via builder", db.len());
            println!("Source: {:?}", db.source());
        }
        Err(e) => {
            println!("No schemas available from any source: {e}");
            println!("(This is expected in a clean build without pre-extracted schemas)");
        }
    }
}
