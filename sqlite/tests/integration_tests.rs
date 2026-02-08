//! Integration tests for the command-schema-sqlite crate.

use command_schema_core::{
    ArgSchema, CommandSchema, FlagSchema, SchemaSource, SubcommandSchema, ValueType,
};
use command_schema_sqlite::{Migration, SchemaQuery};
use rusqlite::Connection;
use std::io::Write;

/// Creates a simple test schema for a command.
fn simple_schema(name: &str, source: SchemaSource) -> CommandSchema {
    let mut schema = CommandSchema::new(name, source);
    schema.description = Some(format!("The {name} command"));
    schema.version = Some("1.0.0".to_string());
    schema
        .global_flags
        .push(FlagSchema::boolean(Some("-v"), Some("--verbose")).with_description("Be verbose"));
    schema
        .global_flags
        .push(FlagSchema::boolean(Some("-q"), Some("--quiet")).with_description("Be quiet"));
    schema
}

/// Creates a complex test schema with nested subcommands, choices, aliases, and relationships.
fn complex_schema() -> CommandSchema {
    let mut schema = CommandSchema::new("git", SchemaSource::Bootstrap);
    schema.description = Some("The stupid content tracker".to_string());
    schema.version = Some("2.43.0".to_string());
    schema.schema_version = Some("1.0.0".to_string());
    schema.confidence = 0.95;

    // Global flags
    schema.global_flags.push(
        FlagSchema::boolean(Some("-v"), Some("--verbose")).with_description("Be verbose"),
    );
    schema
        .global_flags
        .push(FlagSchema::with_value(None, Some("--git-dir"), ValueType::Directory));

    // Global positional arg
    schema
        .positional
        .push(ArgSchema::optional("pathspec", ValueType::File));

    // Subcommand: commit
    let mut commit = SubcommandSchema::new("commit");
    commit.description = Some("Record changes to the repository".to_string());
    commit.aliases = vec!["ci".to_string()];

    commit.flags.push(
        FlagSchema::with_value(Some("-m"), Some("--message"), ValueType::String)
            .with_description("Commit message"),
    );
    commit
        .flags
        .push(FlagSchema::boolean(Some("-a"), Some("--all")).with_description("Stage all changes"));
    commit.flags.push(
        FlagSchema::boolean(None, Some("--amend")).with_description("Amend previous commit"),
    );

    // Flag with choices
    commit.flags.push(FlagSchema::with_value(
        None,
        Some("--cleanup"),
        ValueType::Choice(vec![
            "strip".to_string(),
            "whitespace".to_string(),
            "verbatim".to_string(),
            "scissors".to_string(),
            "default".to_string(),
        ]),
    ));

    // Flag relationships: --all conflicts with --amend
    commit.flags[1].conflicts_with = vec!["--amend".to_string()];
    commit.flags[2].requires = vec!["--message".to_string()];

    commit
        .positional
        .push(ArgSchema::optional("file", ValueType::File).allow_multiple());

    schema.subcommands.push(commit);

    // Subcommand: remote (with nested subcommands)
    let mut remote = SubcommandSchema::new("remote");
    remote.description = Some("Manage set of tracked repositories".to_string());

    let mut remote_add = SubcommandSchema::new("add");
    remote_add.description = Some("Add a remote".to_string());
    remote_add
        .positional
        .push(ArgSchema::required("name", ValueType::String));
    remote_add
        .positional
        .push(ArgSchema::required("url", ValueType::Url));
    remote_add
        .flags
        .push(FlagSchema::boolean(Some("-f"), Some("--fetch")));

    let mut remote_remove = SubcommandSchema::new("remove");
    remote_remove.description = Some("Remove a remote".to_string());
    remote_remove.aliases = vec!["rm".to_string()];
    remote_remove
        .positional
        .push(ArgSchema::required("name", ValueType::Remote));

    // Three levels deep: remote > show > info
    let mut remote_show = SubcommandSchema::new("show");
    remote_show.description = Some("Show information about remote".to_string());
    remote_show
        .positional
        .push(ArgSchema::optional("name", ValueType::Remote));

    let mut remote_show_info = SubcommandSchema::new("info");
    remote_show_info.description = Some("Show detailed remote info".to_string());
    remote_show_info
        .flags
        .push(FlagSchema::boolean(None, Some("--all")));

    remote_show.subcommands.push(remote_show_info);

    remote.subcommands.push(remote_add);
    remote.subcommands.push(remote_remove);
    remote.subcommands.push(remote_show);

    schema.subcommands.push(remote);

    // Subcommand: branch with choice arg
    let mut branch = SubcommandSchema::new("branch");
    branch.description = Some("List, create, or delete branches".to_string());
    branch.flags.push(FlagSchema::with_value(
        None,
        Some("--sort"),
        ValueType::Choice(vec![
            "refname".to_string(),
            "committerdate".to_string(),
            "authordate".to_string(),
        ]),
    ));
    branch
        .positional
        .push(ArgSchema::optional("branch-name", ValueType::Branch));

    schema.subcommands.push(branch);

    schema
}

/// Helper to set up a migration and run up().
fn setup_migration() -> Migration {
    let conn = Connection::open_in_memory().unwrap();
    let mut migration = Migration::new(conn, "cs_").unwrap();
    migration.up().unwrap();
    migration
}

/// Helper to set up a SchemaQuery with tables already created.
fn setup_query() -> SchemaQuery {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

    // Create tables first
    let schema_sql = command_schema_sqlite::Migration::new(
        Connection::open_in_memory().unwrap(),
        "cs_",
    )
    .unwrap();
    // We need to create tables on the actual connection
    drop(schema_sql);

    let mut migration = Migration::new(conn, "cs_").unwrap();
    migration.up().unwrap();

    let conn = migration.into_connection();
    SchemaQuery::new(conn, "cs_").unwrap()
}

// =============================================================================
// Full Migration Lifecycle Tests
// =============================================================================

#[test]
fn test_migration_lifecycle() {
    let mut migration = setup_migration();

    // Verify tables exist with 0 rows
    let status = migration.status().unwrap();
    assert!(status.tables_exist);
    assert_eq!(status.command_count, 0);
    assert_eq!(status.flag_count, 0);
    assert_eq!(status.subcommand_count, 0);
    assert_eq!(status.arg_count, 0);

    // Drop and verify
    migration.down().unwrap();
    let status = migration.status().unwrap();
    assert!(!status.tables_exist);
}

#[test]
fn test_seed_from_directory() {
    let dir = std::env::temp_dir().join("cs_sqlite_test_seed");
    std::fs::create_dir_all(&dir).unwrap();

    // Write test schemas
    let schema1 = simple_schema("curl", SchemaSource::HelpCommand);
    let schema2 = simple_schema("wget", SchemaSource::ManPage);

    for schema in [&schema1, &schema2] {
        let path = dir.join(format!("{}.json", schema.command));
        let mut f = std::fs::File::create(&path).unwrap();
        serde_json::to_writer_pretty(&mut f, schema).unwrap();
        f.flush().unwrap();
    }

    let mut migration = setup_migration();
    let report = migration.seed(&dir).unwrap();

    assert_eq!(report.commands_inserted, 2);
    assert!(report.flags_inserted >= 4); // 2 flags per command

    let status = migration.status().unwrap();
    assert_eq!(status.command_count, 2);
    assert!(status.flag_count >= 4);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_refresh() {
    let dir = std::env::temp_dir().join("cs_sqlite_test_refresh");
    std::fs::create_dir_all(&dir).unwrap();

    let schema = simple_schema("curl", SchemaSource::HelpCommand);
    let path = dir.join("curl.json");
    let mut f = std::fs::File::create(&path).unwrap();
    serde_json::to_writer_pretty(&mut f, &schema).unwrap();
    f.flush().unwrap();

    let mut migration = setup_migration();
    migration.seed(&dir).unwrap();

    // Refresh should drop, recreate, and reseed
    let report = migration.refresh(&dir).unwrap();
    assert_eq!(report.commands_inserted, 1);

    let status = migration.status().unwrap();
    assert_eq!(status.command_count, 1);

    std::fs::remove_dir_all(&dir).ok();
}

// =============================================================================
// Round-trip Conversion Tests
// =============================================================================

#[test]
fn test_round_trip_simple_schema() {
    let mut query = setup_query();
    let original = simple_schema("curl", SchemaSource::HelpCommand);

    query.insert_schema(&original).unwrap();
    let loaded = query.get_schema("curl").unwrap().unwrap();

    assert_eq!(loaded.command, original.command);
    assert_eq!(loaded.description, original.description);
    assert_eq!(loaded.version, original.version);
    assert_eq!(loaded.source, original.source);
    assert_eq!(loaded.global_flags.len(), original.global_flags.len());

    for (orig, load) in original.global_flags.iter().zip(loaded.global_flags.iter()) {
        assert_eq!(orig.short, load.short);
        assert_eq!(orig.long, load.long);
        assert_eq!(orig.value_type, load.value_type);
        assert_eq!(orig.takes_value, load.takes_value);
        assert_eq!(orig.description, load.description);
        assert_eq!(orig.multiple, load.multiple);
    }
}

#[test]
fn test_round_trip_complex_schema() {
    let mut query = setup_query();
    let original = complex_schema();

    query.insert_schema(&original).unwrap();
    let loaded = query.get_schema("git").unwrap().unwrap();

    // Top-level fields
    assert_eq!(loaded.command, "git");
    assert_eq!(loaded.description, original.description);
    assert_eq!(loaded.version, original.version);
    assert_eq!(loaded.schema_version, original.schema_version);
    assert_eq!(loaded.source, SchemaSource::Bootstrap);
    assert!((loaded.confidence - 0.95).abs() < f64::EPSILON);

    // Global flags
    assert_eq!(loaded.global_flags.len(), 2);
    assert_eq!(loaded.global_flags[0].short, Some("-v".to_string()));
    assert_eq!(loaded.global_flags[1].long, Some("--git-dir".to_string()));
    assert_eq!(loaded.global_flags[1].value_type, ValueType::Directory);

    // Global positional args
    assert_eq!(loaded.positional.len(), 1);
    assert_eq!(loaded.positional[0].name, "pathspec");
    assert_eq!(loaded.positional[0].value_type, ValueType::File);

    // Subcommands
    assert_eq!(loaded.subcommands.len(), 3);

    // commit subcommand
    let commit = &loaded.subcommands[0];
    assert_eq!(commit.name, "commit");
    assert_eq!(commit.aliases, vec!["ci"]);
    assert_eq!(commit.flags.len(), 4);
    assert_eq!(
        commit.description,
        Some("Record changes to the repository".to_string())
    );

    // commit --cleanup flag with choices
    let cleanup = &commit.flags[3];
    assert_eq!(cleanup.long, Some("--cleanup".to_string()));
    match &cleanup.value_type {
        ValueType::Choice(choices) => {
            assert_eq!(choices.len(), 5);
            assert_eq!(choices[0], "strip");
            assert_eq!(choices[4], "default");
        }
        _ => panic!("Expected Choice value type for --cleanup"),
    }

    // Flag relationships
    let all_flag = &commit.flags[1]; // --all
    assert_eq!(all_flag.conflicts_with, vec!["--amend"]);

    let amend_flag = &commit.flags[2]; // --amend
    assert_eq!(amend_flag.requires, vec!["--message"]);

    // commit positional args
    assert_eq!(commit.positional.len(), 1);
    assert_eq!(commit.positional[0].name, "file");
    assert!(commit.positional[0].multiple);

    // remote subcommand with nested subcommands
    let remote = &loaded.subcommands[1];
    assert_eq!(remote.name, "remote");
    assert_eq!(remote.subcommands.len(), 3); // add, remove, show

    let remote_add = &remote.subcommands[0];
    assert_eq!(remote_add.name, "add");
    assert_eq!(remote_add.positional.len(), 2);
    assert_eq!(remote_add.positional[0].name, "name");
    assert_eq!(remote_add.positional[1].value_type, ValueType::Url);
    assert_eq!(remote_add.flags.len(), 1);

    let remote_remove = &remote.subcommands[1];
    assert_eq!(remote_remove.name, "remove");
    assert_eq!(remote_remove.aliases, vec!["rm"]);
    assert_eq!(remote_remove.positional[0].value_type, ValueType::Remote);

    // Three levels deep: remote > show > info
    let remote_show = &remote.subcommands[2];
    assert_eq!(remote_show.name, "show");
    assert_eq!(remote_show.subcommands.len(), 1);
    let info = &remote_show.subcommands[0];
    assert_eq!(info.name, "info");
    assert_eq!(info.flags.len(), 1);
    assert_eq!(info.flags[0].long, Some("--all".to_string()));

    // branch subcommand with choice flag
    let branch = &loaded.subcommands[2];
    assert_eq!(branch.name, "branch");
    match &branch.flags[0].value_type {
        ValueType::Choice(choices) => {
            assert_eq!(choices.len(), 3);
            assert!(choices.contains(&"refname".to_string()));
        }
        _ => panic!("Expected Choice value type for --sort"),
    }
    assert_eq!(branch.positional[0].value_type, ValueType::Branch);
}

#[test]
fn test_round_trip_all_value_types() {
    let mut query = setup_query();

    let mut schema = CommandSchema::new("types_test", SchemaSource::Learned);
    schema.global_flags.push(FlagSchema::boolean(None, Some("--bool-flag")));
    schema.global_flags.push(FlagSchema::with_value(None, Some("--string-flag"), ValueType::String));
    schema.global_flags.push(FlagSchema::with_value(None, Some("--number-flag"), ValueType::Number));
    schema.global_flags.push(FlagSchema::with_value(None, Some("--file-flag"), ValueType::File));
    schema.global_flags.push(FlagSchema::with_value(None, Some("--dir-flag"), ValueType::Directory));
    schema.global_flags.push(FlagSchema::with_value(None, Some("--url-flag"), ValueType::Url));
    schema.global_flags.push(FlagSchema::with_value(None, Some("--branch-flag"), ValueType::Branch));
    schema.global_flags.push(FlagSchema::with_value(None, Some("--remote-flag"), ValueType::Remote));
    schema.global_flags.push(FlagSchema::with_value(
        None,
        Some("--choice-flag"),
        ValueType::Choice(vec!["a".to_string(), "b".to_string()]),
    ));
    schema.global_flags.push(FlagSchema::with_value(None, Some("--any-flag"), ValueType::Any));

    query.insert_schema(&schema).unwrap();
    let loaded = query.get_schema("types_test").unwrap().unwrap();

    assert_eq!(loaded.global_flags.len(), 10);
    assert_eq!(loaded.global_flags[0].value_type, ValueType::Bool);
    assert_eq!(loaded.global_flags[1].value_type, ValueType::String);
    assert_eq!(loaded.global_flags[2].value_type, ValueType::Number);
    assert_eq!(loaded.global_flags[3].value_type, ValueType::File);
    assert_eq!(loaded.global_flags[4].value_type, ValueType::Directory);
    assert_eq!(loaded.global_flags[5].value_type, ValueType::Url);
    assert_eq!(loaded.global_flags[6].value_type, ValueType::Branch);
    assert_eq!(loaded.global_flags[7].value_type, ValueType::Remote);
    assert_eq!(
        loaded.global_flags[8].value_type,
        ValueType::Choice(vec!["a".to_string(), "b".to_string()])
    );
    assert_eq!(loaded.global_flags[9].value_type, ValueType::Any);
}

// =============================================================================
// Query Operation Tests
// =============================================================================

#[test]
fn test_get_all_schemas() {
    let mut query = setup_query();

    query
        .insert_schema(&simple_schema("curl", SchemaSource::HelpCommand))
        .unwrap();
    query
        .insert_schema(&simple_schema("wget", SchemaSource::ManPage))
        .unwrap();
    query
        .insert_schema(&simple_schema("cargo", SchemaSource::Bootstrap))
        .unwrap();

    let all = query.get_all_schemas().unwrap();
    assert_eq!(all.len(), 3);

    // Should be sorted by name
    assert_eq!(all[0].command, "cargo");
    assert_eq!(all[1].command, "curl");
    assert_eq!(all[2].command, "wget");
}

#[test]
fn test_get_by_source() {
    let mut query = setup_query();

    query
        .insert_schema(&simple_schema("curl", SchemaSource::HelpCommand))
        .unwrap();
    query
        .insert_schema(&simple_schema("wget", SchemaSource::HelpCommand))
        .unwrap();
    query
        .insert_schema(&simple_schema("cargo", SchemaSource::Bootstrap))
        .unwrap();
    query
        .insert_schema(&simple_schema("rustc", SchemaSource::ManPage))
        .unwrap();

    let help_schemas = query.get_by_source(SchemaSource::HelpCommand).unwrap();
    assert_eq!(help_schemas.len(), 2);

    let bootstrap_schemas = query.get_by_source(SchemaSource::Bootstrap).unwrap();
    assert_eq!(bootstrap_schemas.len(), 1);
    assert_eq!(bootstrap_schemas[0].command, "cargo");

    let learned_schemas = query.get_by_source(SchemaSource::Learned).unwrap();
    assert_eq!(learned_schemas.len(), 0);
}

#[test]
fn test_update_schema() {
    let mut query = setup_query();

    let mut original = simple_schema("curl", SchemaSource::HelpCommand);
    query.insert_schema(&original).unwrap();

    // Modify and update
    original.description = Some("Updated description".to_string());
    original.global_flags.push(
        FlagSchema::with_value(Some("-o"), Some("--output"), ValueType::File)
            .with_description("Write to file"),
    );

    query.update_schema(&original).unwrap();

    let loaded = query.get_schema("curl").unwrap().unwrap();
    assert_eq!(loaded.description, Some("Updated description".to_string()));
    assert_eq!(loaded.global_flags.len(), 3); // Original 2 + new 1
}

#[test]
fn test_update_nonexistent_schema() {
    let mut query = setup_query();
    let schema = simple_schema("nonexistent", SchemaSource::Bootstrap);
    assert!(query.update_schema(&schema).is_err());
}

#[test]
fn test_delete_schema() {
    let mut query = setup_query();

    query
        .insert_schema(&simple_schema("curl", SchemaSource::HelpCommand))
        .unwrap();
    assert!(query.get_schema("curl").unwrap().is_some());

    query.delete_schema("curl").unwrap();
    assert!(query.get_schema("curl").unwrap().is_none());
}

#[test]
fn test_delete_nonexistent_schema() {
    let query = setup_query();
    assert!(query.delete_schema("nonexistent").is_err());
}

#[test]
fn test_get_nonexistent_schema() {
    let query = setup_query();
    assert!(query.get_schema("nonexistent").unwrap().is_none());
}

// =============================================================================
// Prefix Isolation Tests
// =============================================================================

#[test]
fn test_prefix_isolation() {
    let conn = Connection::open_in_memory().unwrap();

    // Create tables with prefix "a_"
    let mut migration_a = Migration::new(conn, "a_").unwrap();
    migration_a.up().unwrap();

    let conn = migration_a.into_connection();

    // Create tables with prefix "b_" on the same connection
    let mut migration_b = Migration::new(conn, "b_").unwrap();
    migration_b.up().unwrap();

    let conn = migration_b.into_connection();

    // Insert into "a_" namespace
    let mut query_a = SchemaQuery::new(conn, "a_").unwrap();
    query_a
        .insert_schema(&simple_schema("curl", SchemaSource::HelpCommand))
        .unwrap();

    // For a proper prefix isolation test, use the same in-memory database
    // by creating both prefix tables and inserting into each
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

    let mut mig_a = Migration::new(conn, "a_").unwrap();
    mig_a.up().unwrap();
    let conn = mig_a.into_connection();

    let mut mig_b = Migration::new(conn, "b_").unwrap();
    mig_b.up().unwrap();
    let mut conn = mig_b.into_connection();

    // Insert schema into prefix "a_"
    {
        let schema = simple_schema("curl", SchemaSource::HelpCommand);
        let tx = conn.transaction().unwrap();
        command_schema_sqlite_test_helpers::insert_into(&tx, "a_", &schema);
        tx.commit().unwrap();
    }

    // Insert different schema into prefix "b_"
    {
        let schema = simple_schema("wget", SchemaSource::ManPage);
        let tx = conn.transaction().unwrap();
        command_schema_sqlite_test_helpers::insert_into(&tx, "b_", &schema);
        tx.commit().unwrap();
    }

    // Query prefix "a_" - should only have curl
    let query_a = SchemaQuery::new(conn, "a_").unwrap();
    let all_a = query_a.get_all_schemas().unwrap();
    assert_eq!(all_a.len(), 1);
    assert_eq!(all_a[0].command, "curl");

    // We can't easily reuse the connection for "b_" queries without
    // into_connection on SchemaQuery, so we verify by checking the
    // "a_" prefix doesn't contain "wget"
    assert!(query_a.get_schema("wget").unwrap().is_none());
}

// Helper module for test utilities that need access to convert internals
mod command_schema_sqlite_test_helpers {
    use command_schema_core::CommandSchema;
    use rusqlite::Connection;

    pub fn insert_into(conn: &Connection, prefix: &str, schema: &CommandSchema) {
        conn.execute(
            &format!(
                "INSERT INTO {prefix}commands (name, description, version, source, confidence) \
                 VALUES (?1, ?2, ?3, ?4, ?5)"
            ),
            rusqlite::params![
                schema.command,
                schema.description,
                schema.version,
                "HelpCommand",
                schema.confidence,
            ],
        )
        .unwrap();

        let command_id = conn.last_insert_rowid();

        for flag in &schema.global_flags {
            conn.execute(
                &format!(
                    "INSERT INTO {prefix}flags (command_id, short, long, value_type, takes_value, description, multiple) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
                ),
                rusqlite::params![
                    command_id,
                    flag.short,
                    flag.long,
                    "Bool",
                    flag.takes_value as i32,
                    flag.description,
                    flag.multiple as i32,
                ],
            )
            .unwrap();
        }
    }
}

// =============================================================================
// Cascade Delete Tests
// =============================================================================

#[test]
fn test_cascade_delete_cleans_up_all_related_data() {
    let mut query = setup_query();
    let schema = complex_schema();

    query.insert_schema(&schema).unwrap();

    // Verify data exists
    let loaded = query.get_schema("git").unwrap().unwrap();
    assert!(!loaded.subcommands.is_empty());
    assert!(!loaded.global_flags.is_empty());

    // Delete should cascade
    query.delete_schema("git").unwrap();

    // Verify everything is cleaned up
    assert!(query.get_schema("git").unwrap().is_none());

    // Verify no orphaned rows (check via direct SQL)
    let conn = query.connection();
    let flag_count: i64 = conn
        .prepare("SELECT COUNT(*) FROM cs_flags")
        .unwrap()
        .query_row([], |row| row.get(0))
        .unwrap();
    assert_eq!(flag_count, 0);

    let sub_count: i64 = conn
        .prepare("SELECT COUNT(*) FROM cs_subcommands")
        .unwrap()
        .query_row([], |row| row.get(0))
        .unwrap();
    assert_eq!(sub_count, 0);

    let arg_count: i64 = conn
        .prepare("SELECT COUNT(*) FROM cs_positional_args")
        .unwrap()
        .query_row([], |row| row.get(0))
        .unwrap();
    assert_eq!(arg_count, 0);

    let choice_count: i64 = conn
        .prepare("SELECT COUNT(*) FROM cs_flag_choices")
        .unwrap()
        .query_row([], |row| row.get(0))
        .unwrap();
    assert_eq!(choice_count, 0);

    let alias_count: i64 = conn
        .prepare("SELECT COUNT(*) FROM cs_subcommand_aliases")
        .unwrap()
        .query_row([], |row| row.get(0))
        .unwrap();
    assert_eq!(alias_count, 0);
}

// =============================================================================
// Multiple Flag Occurrence Tests
// =============================================================================

#[test]
fn test_multiple_flag_round_trip() {
    let mut query = setup_query();
    let mut schema = CommandSchema::new("test", SchemaSource::Bootstrap);
    schema
        .global_flags
        .push(FlagSchema::with_value(Some("-I"), Some("--include"), ValueType::String).allow_multiple());

    query.insert_schema(&schema).unwrap();
    let loaded = query.get_schema("test").unwrap().unwrap();

    assert!(loaded.global_flags[0].multiple);
}

// =============================================================================
// Empty Schema Tests
// =============================================================================

#[test]
fn test_empty_schema_round_trip() {
    let mut query = setup_query();
    let schema = CommandSchema::new("empty", SchemaSource::Bootstrap);

    query.insert_schema(&schema).unwrap();
    let loaded = query.get_schema("empty").unwrap().unwrap();

    assert_eq!(loaded.command, "empty");
    assert!(loaded.global_flags.is_empty());
    assert!(loaded.subcommands.is_empty());
    assert!(loaded.positional.is_empty());
}
