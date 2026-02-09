//! SQLite migration and query workflow example.
//!
//! Demonstrates the complete SQLite lifecycle: creating tables, seeding
//! from JSON, querying schemas, inserting learned schemas, and cleanup.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p command-schema-examples --example sqlite_migration
//! ```

use std::io::Write;

use command_schema_core::{
    ArgSchema, CommandSchema, FlagSchema, SchemaSource, SubcommandSchema, ValueType,
};
use command_schema_sqlite::{Migration, SchemaQuery};
use rusqlite::Connection;

fn main() {
    // === Step 1: Set up a temporary schema directory ===
    let schema_dir = std::env::temp_dir().join("cs_sqlite_example_schemas");
    std::fs::create_dir_all(&schema_dir).unwrap();

    // Write sample schemas as JSON files
    write_schema_file(&schema_dir, &create_git_schema());
    write_schema_file(&schema_dir, &create_curl_schema());

    // === Step 2: Create SQLite database and run migrations ===
    println!("=== Migration ===");
    let conn = Connection::open_in_memory().unwrap();
    let mut migration = Migration::new(conn, "cs_").unwrap();

    // Check initial status
    let status = migration.status().unwrap();
    println!("Before migration: tables_exist={}", status.tables_exist);

    // Create tables
    migration.up().unwrap();
    let status = migration.status().unwrap();
    println!("After up(): tables_exist={}", status.tables_exist);

    // === Step 3: Seed from JSON schema files ===
    println!("\n=== Seeding ===");
    let report = migration.seed(&schema_dir).unwrap();
    println!("Seed report:");
    println!("  Commands inserted: {}", report.commands_inserted);
    println!("  Flags inserted: {}", report.flags_inserted);
    println!("  Subcommands inserted: {}", report.subcommands_inserted);
    println!("  Args inserted: {}", report.args_inserted);
    println!("  Choices inserted: {}", report.choices_inserted);
    println!("  Aliases inserted: {}", report.aliases_inserted);
    println!(
        "  Relationships inserted: {}",
        report.relationships_inserted
    );

    let status = migration.status().unwrap();
    println!("\nDatabase status:");
    println!("  Commands: {}", status.command_count);
    println!("  Flags: {}", status.flag_count);
    println!("  Subcommands: {}", status.subcommand_count);

    // === Step 4: Query schemas ===
    println!("\n=== Querying ===");
    let conn = migration.into_connection();
    let mut query = SchemaQuery::new(conn, "cs_").unwrap();

    // Get a single schema
    if let Some(git) = query.get_schema("git").unwrap() {
        println!("git schema:");
        println!("  Source: {:?}", git.source);
        println!("  Global flags: {}", git.global_flags.len());
        println!("  Subcommands: {}", git.subcommands.len());
        for sub in &git.subcommands {
            println!("    {} ({} flags)", sub.name, sub.flags.len());
        }
    }

    // Get all schemas
    let all = query.get_all_schemas().unwrap();
    println!("\nAll schemas ({}):", all.len());
    for schema in &all {
        println!("  {} ({:?})", schema.command, schema.source);
    }

    // Filter by source
    let bootstrap = query.get_by_source(SchemaSource::Bootstrap).unwrap();
    println!("\nBootstrap schemas: {}", bootstrap.len());

    // === Step 5: Insert a learned schema at runtime ===
    println!("\n=== Runtime learning ===");
    let mut learned = CommandSchema::new("mycli", SchemaSource::Learned);
    learned.global_flags.push(
        FlagSchema::boolean(Some("-v"), Some("--verbose")).with_description("Verbose output"),
    );
    query.insert_schema(&learned).unwrap();
    println!("Inserted learned schema for 'mycli'");

    let learned_schemas = query.get_by_source(SchemaSource::Learned).unwrap();
    println!("Learned schemas: {}", learned_schemas.len());

    // === Step 6: Update an existing schema ===
    learned.global_flags.push(
        FlagSchema::with_value(Some("-o"), Some("--output"), ValueType::File)
            .with_description("Output file"),
    );
    query.update_schema(&learned).unwrap();
    println!(
        "Updated 'mycli' schema (now {} flags)",
        query
            .get_schema("mycli")
            .unwrap()
            .unwrap()
            .global_flags
            .len()
    );

    // === Step 7: Delete a schema ===
    query.delete_schema("mycli").unwrap();
    println!("Deleted 'mycli' schema");
    assert!(query.get_schema("mycli").unwrap().is_none());

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
    schema.version = Some("2.43.0".into());
    schema
        .global_flags
        .push(FlagSchema::boolean(Some("-v"), Some("--verbose")).with_description("Be verbose"));
    schema.subcommands.push(
        SubcommandSchema::new("commit")
            .with_flag(
                FlagSchema::with_value(Some("-m"), Some("--message"), ValueType::String)
                    .with_description("Commit message"),
            )
            .with_flag(FlagSchema::boolean(Some("-a"), Some("--all"))),
    );
    schema.subcommands.push(
        SubcommandSchema::new("push")
            .with_arg(ArgSchema::optional("remote", ValueType::Remote))
            .with_arg(ArgSchema::optional("branch", ValueType::Branch)),
    );
    schema
}

fn create_curl_schema() -> CommandSchema {
    let mut schema = CommandSchema::new("curl", SchemaSource::HelpCommand);
    schema.description = Some("Transfer data from or to a server".into());
    schema.global_flags.push(
        FlagSchema::boolean(Some("-v"), Some("--verbose"))
            .with_description("Make the operation more talkative"),
    );
    schema.global_flags.push(
        FlagSchema::with_value(Some("-o"), Some("--output"), ValueType::File)
            .with_description("Write to file instead of stdout"),
    );
    schema
        .positional
        .push(ArgSchema::required("url", ValueType::Url));
    schema
}
