//! Static database loading example.
//!
//! Demonstrates how to load pre-extracted command schemas from a directory
//! of JSON files using `SchemaDatabase`, perform O(1) lookups, and iterate
//! over all loaded commands.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p command-schema-examples --example load_static_db
//! ```
//!
//! This example creates temporary schema files to demonstrate the API.

use std::io::Write;

use command_schema_core::{
    CommandSchema, FlagSchema, SchemaSource, SubcommandSchema, ValueType,
};
use command_schema_db::SchemaDatabase;

fn main() {
    // Create a temporary directory with sample schema files
    let dir = std::env::temp_dir().join("command_schema_example_db");
    std::fs::create_dir_all(&dir).unwrap();

    // Create sample schemas
    let schemas = vec![
        create_git_schema(),
        create_docker_schema(),
        create_cargo_schema(),
    ];

    for schema in &schemas {
        let path = dir.join(format!("{}.json", schema.command));
        let mut file = std::fs::File::create(&path).unwrap();
        serde_json::to_writer_pretty(&mut file, schema).unwrap();
        file.flush().unwrap();
    }

    // Load all schemas from the directory
    let start = std::time::Instant::now();
    let db = SchemaDatabase::from_dir(&dir).unwrap();
    let elapsed = start.elapsed();

    println!("Loaded {} schemas in {:.2?}", db.len(), elapsed);
    println!();

    // O(1) lookup by command name
    if let Some(git) = db.get("git") {
        println!("git schema:");
        println!("  Source: {:?}", git.source);
        println!("  Confidence: {:.2}", git.confidence);
        println!("  Global flags: {}", git.global_flags.len());
        println!("  Subcommands: {}", git.subcommands.len());

        // Look up a specific subcommand
        if let Some(commit) = git.find_subcommand("commit") {
            println!("  git commit flags: {}", commit.flags.len());
        }

        // Get all flags for a subcommand (global + subcommand-specific)
        let push_flags = git.flags_for_subcommand("push");
        println!("  git push total flags (global + local): {}", push_flags.len());
    }

    println!();

    // Iterate over all commands
    println!("All loaded commands:");
    let mut commands: Vec<&str> = db.commands().collect();
    commands.sort();
    for name in commands {
        let schema = db.get(name).unwrap();
        println!(
            "  {name}: {} flags, {} subcommands",
            schema.global_flags.len(),
            schema.subcommands.len()
        );
    }

    // Builder pattern with fallback chain
    println!();
    println!("Builder pattern with fallback:");
    let db = SchemaDatabase::builder()
        .from_dir(&dir)                              // Try directory first
        .from_bundle("/nonexistent/bundle.json")     // Falls back to bundle
        .build()
        .unwrap();
    println!("  Loaded {} schemas via builder", db.len());

    // Cleanup
    std::fs::remove_dir_all(&dir).ok();
}

fn create_git_schema() -> CommandSchema {
    let mut schema = CommandSchema::new("git", SchemaSource::Bootstrap);
    schema.description = Some("The stupid content tracker".into());
    schema.global_flags.push(
        FlagSchema::boolean(Some("-v"), Some("--verbose"))
            .with_description("Be verbose"),
    );
    schema.global_flags.push(
        FlagSchema::with_value(Some("-C"), Some("--directory"), ValueType::Directory)
            .with_description("Run as if git was started in <path>"),
    );
    schema.subcommands.push(
        SubcommandSchema::new("commit")
            .with_flag(FlagSchema::with_value(
                Some("-m"),
                Some("--message"),
                ValueType::String,
            ))
            .with_flag(FlagSchema::boolean(Some("-a"), Some("--all"))),
    );
    schema.subcommands.push(SubcommandSchema::new("push"));
    schema.subcommands.push(SubcommandSchema::new("pull"));
    schema
}

fn create_docker_schema() -> CommandSchema {
    let mut schema = CommandSchema::new("docker", SchemaSource::HelpCommand);
    schema.description = Some("A self-sufficient runtime for containers".into());
    schema.global_flags.push(
        FlagSchema::boolean(Some("-D"), Some("--debug"))
            .with_description("Enable debug mode"),
    );
    schema.subcommands.push(SubcommandSchema::new("run"));
    schema.subcommands.push(SubcommandSchema::new("build"));
    schema.subcommands.push(SubcommandSchema::new("ps"));
    schema
}

fn create_cargo_schema() -> CommandSchema {
    let mut schema = CommandSchema::new("cargo", SchemaSource::HelpCommand);
    schema.description = Some("Rust's package manager".into());
    schema.global_flags.push(
        FlagSchema::boolean(Some("-v"), Some("--verbose"))
            .with_description("Use verbose output"),
    );
    schema.subcommands.push(SubcommandSchema::new("build"));
    schema.subcommands.push(SubcommandSchema::new("test"));
    schema.subcommands.push(SubcommandSchema::new("run"));
    schema
}
