//! SQL schema generation with customizable table prefixes.
//!
//! Generates normalized `CREATE TABLE` and `CREATE INDEX` statements for
//! storing command schemas in SQLite. All table names are prefixed with a
//! configurable string to allow multiple isolated schema sets in the same
//! database.
//!
//! # Table structure
//!
//! The normalized schema consists of 8 tables:
//!
//! - `{prefix}commands` — top-level command metadata
//! - `{prefix}subcommands` — nested subcommands with parent tracking
//! - `{prefix}flags` — flags with scope (global or subcommand-specific)
//! - `{prefix}positional_args` — positional arguments with ordering
//! - `{prefix}flag_choices` — allowed values for `Choice` flags
//! - `{prefix}arg_choices` — allowed values for `Choice` args
//! - `{prefix}subcommand_aliases` — alternative names for subcommands
//! - `{prefix}flag_relationships` — conflicts/requires between flags
//!
//! # Custom prefix
//!
//! Prefixes must contain only alphanumeric characters and underscores.
//! This enables multiple isolated schema sets (e.g., `prod_`, `test_`)
//! within the same SQLite database.

use crate::error::{Result, SqliteError};

/// Validates that a table prefix contains only alphanumeric characters and underscores.
pub(crate) fn validate_prefix(prefix: &str) -> Result<()> {
    if prefix.is_empty() {
        return Err(SqliteError::InvalidPrefix(prefix.to_string()));
    }
    if !prefix.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(SqliteError::InvalidPrefix(prefix.to_string()));
    }
    Ok(())
}

/// Generates the complete SQL schema for all tables with the given prefix.
///
/// The prefix is prepended to every table and index name, enabling multiple
/// isolated schema sets within the same SQLite database.
///
/// # Errors
///
/// Returns [`SqliteError::InvalidPrefix`] if the prefix contains characters
/// other than alphanumerics and underscores, or if it is empty.
pub fn generate_schema_sql(prefix: &str) -> Result<String> {
    validate_prefix(prefix)?;

    let sql = format!(
        r#"
CREATE TABLE IF NOT EXISTS {prefix}commands (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    version TEXT,
    source TEXT NOT NULL DEFAULT 'HelpCommand',
    confidence REAL NOT NULL DEFAULT 1.0,
    schema_version TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS {prefix}subcommands (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    command_id INTEGER NOT NULL,
    parent_id INTEGER,
    name TEXT NOT NULL,
    description TEXT,
    FOREIGN KEY (command_id) REFERENCES {prefix}commands(id) ON DELETE CASCADE,
    FOREIGN KEY (parent_id) REFERENCES {prefix}subcommands(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS {prefix}flags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    command_id INTEGER NOT NULL,
    subcommand_id INTEGER,
    short TEXT,
    long TEXT,
    value_type TEXT NOT NULL DEFAULT 'Bool',
    takes_value INTEGER NOT NULL DEFAULT 0,
    description TEXT,
    multiple INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (command_id) REFERENCES {prefix}commands(id) ON DELETE CASCADE,
    FOREIGN KEY (subcommand_id) REFERENCES {prefix}subcommands(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS {prefix}positional_args (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    command_id INTEGER,
    subcommand_id INTEGER,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    value_type TEXT NOT NULL DEFAULT 'Any',
    required INTEGER NOT NULL DEFAULT 0,
    multiple INTEGER NOT NULL DEFAULT 0,
    description TEXT,
    CHECK ((command_id IS NOT NULL AND subcommand_id IS NULL) OR (command_id IS NULL AND subcommand_id IS NOT NULL)),
    FOREIGN KEY (command_id) REFERENCES {prefix}commands(id) ON DELETE CASCADE,
    FOREIGN KEY (subcommand_id) REFERENCES {prefix}subcommands(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS {prefix}flag_choices (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    flag_id INTEGER NOT NULL,
    choice TEXT NOT NULL,
    FOREIGN KEY (flag_id) REFERENCES {prefix}flags(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS {prefix}arg_choices (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    arg_id INTEGER NOT NULL,
    choice TEXT NOT NULL,
    FOREIGN KEY (arg_id) REFERENCES {prefix}positional_args(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS {prefix}subcommand_aliases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subcommand_id INTEGER NOT NULL,
    alias TEXT NOT NULL,
    FOREIGN KEY (subcommand_id) REFERENCES {prefix}subcommands(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS {prefix}flag_relationships (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    flag_id INTEGER NOT NULL,
    related_flag_id INTEGER NOT NULL,
    relationship_type TEXT NOT NULL CHECK (relationship_type IN ('conflicts', 'requires')),
    FOREIGN KEY (flag_id) REFERENCES {prefix}flags(id) ON DELETE CASCADE,
    FOREIGN KEY (related_flag_id) REFERENCES {prefix}flags(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_{prefix}flags_command ON {prefix}flags(command_id);
CREATE INDEX IF NOT EXISTS idx_{prefix}flags_subcommand ON {prefix}flags(subcommand_id);
CREATE INDEX IF NOT EXISTS idx_{prefix}subcommands_command ON {prefix}subcommands(command_id);
CREATE INDEX IF NOT EXISTS idx_{prefix}subcommands_parent ON {prefix}subcommands(parent_id);
CREATE INDEX IF NOT EXISTS idx_{prefix}positional_command ON {prefix}positional_args(command_id);
CREATE INDEX IF NOT EXISTS idx_{prefix}positional_subcommand ON {prefix}positional_args(subcommand_id);
CREATE INDEX IF NOT EXISTS idx_{prefix}commands_source ON {prefix}commands(source);
"#,
        prefix = prefix
    );

    Ok(sql)
}

/// Generates SQL to drop all schema tables in reverse dependency order.
///
/// # Errors
///
/// Returns [`SqliteError::InvalidPrefix`] if the prefix is invalid.
pub fn generate_drop_sql(prefix: &str) -> Result<String> {
    validate_prefix(prefix)?;

    let sql = format!(
        r#"
DROP TABLE IF EXISTS {prefix}flag_relationships;
DROP TABLE IF EXISTS {prefix}flag_choices;
DROP TABLE IF EXISTS {prefix}arg_choices;
DROP TABLE IF EXISTS {prefix}subcommand_aliases;
DROP TABLE IF EXISTS {prefix}positional_args;
DROP TABLE IF EXISTS {prefix}flags;
DROP TABLE IF EXISTS {prefix}subcommands;
DROP TABLE IF EXISTS {prefix}commands;
"#,
        prefix = prefix
    );

    Ok(sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_prefix() {
        assert!(validate_prefix("cs_").is_ok());
        assert!(validate_prefix("test123").is_ok());
        assert!(validate_prefix("A_B_C").is_ok());
    }

    #[test]
    fn test_invalid_prefix_empty() {
        assert!(validate_prefix("").is_err());
    }

    #[test]
    fn test_invalid_prefix_special_chars() {
        assert!(validate_prefix("drop;--").is_err());
        assert!(validate_prefix("hello world").is_err());
        assert!(validate_prefix("test-prefix").is_err());
    }

    #[test]
    fn test_generate_schema_sql_contains_tables() {
        let sql = generate_schema_sql("cs_").unwrap();
        assert!(sql.contains("cs_commands"));
        assert!(sql.contains("cs_subcommands"));
        assert!(sql.contains("cs_flags"));
        assert!(sql.contains("cs_positional_args"));
        assert!(sql.contains("cs_flag_choices"));
        assert!(sql.contains("cs_arg_choices"));
        assert!(sql.contains("cs_subcommand_aliases"));
        assert!(sql.contains("cs_flag_relationships"));
    }

    #[test]
    fn test_generate_schema_sql_contains_indexes() {
        let sql = generate_schema_sql("cs_").unwrap();
        assert!(sql.contains("idx_cs_flags_command"));
        assert!(sql.contains("idx_cs_flags_subcommand"));
        assert!(sql.contains("idx_cs_subcommands_command"));
        assert!(sql.contains("idx_cs_subcommands_parent"));
        assert!(sql.contains("idx_cs_positional_command"));
        assert!(sql.contains("idx_cs_positional_subcommand"));
        // commands.name has a UNIQUE constraint, which implicitly creates an index
        assert!(!sql.contains("idx_cs_commands_name"));
        assert!(sql.contains("idx_cs_commands_source"));
    }

    #[test]
    fn test_generate_drop_sql_contains_all_tables() {
        let sql = generate_drop_sql("cs_").unwrap();
        assert!(sql.contains("DROP TABLE IF EXISTS cs_commands"));
        assert!(sql.contains("DROP TABLE IF EXISTS cs_flags"));
        assert!(sql.contains("DROP TABLE IF EXISTS cs_subcommands"));
        assert!(sql.contains("DROP TABLE IF EXISTS cs_positional_args"));
        assert!(sql.contains("DROP TABLE IF EXISTS cs_flag_choices"));
        assert!(sql.contains("DROP TABLE IF EXISTS cs_arg_choices"));
        assert!(sql.contains("DROP TABLE IF EXISTS cs_subcommand_aliases"));
        assert!(sql.contains("DROP TABLE IF EXISTS cs_flag_relationships"));
    }

    #[test]
    fn test_generate_drop_sql_invalid_prefix() {
        assert!(generate_drop_sql("").is_err());
    }

    #[test]
    fn test_positional_args_check_constraint_exactly_one_scope() {
        let sql = generate_schema_sql("t_").unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(&sql).unwrap();

        // Insert a command and subcommand for FK references
        conn.execute(
            "INSERT INTO t_commands (name, source) VALUES ('cmd', 'Bootstrap')",
            [],
        )
        .unwrap();
        let cmd_id: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO t_subcommands (command_id, name) VALUES (?1, 'sub')",
            [cmd_id],
        )
        .unwrap();
        let sub_id: i64 = conn.last_insert_rowid();

        // command_id only — should succeed
        assert!(conn
            .execute(
                "INSERT INTO t_positional_args (command_id, subcommand_id, position, name) VALUES (?1, NULL, 0, 'a')",
                rusqlite::params![cmd_id],
            )
            .is_ok());

        // subcommand_id only — should succeed
        assert!(conn
            .execute(
                "INSERT INTO t_positional_args (command_id, subcommand_id, position, name) VALUES (NULL, ?1, 0, 'b')",
                rusqlite::params![sub_id],
            )
            .is_ok());

        // Both set — should fail
        assert!(conn
            .execute(
                "INSERT INTO t_positional_args (command_id, subcommand_id, position, name) VALUES (?1, ?2, 0, 'c')",
                rusqlite::params![cmd_id, sub_id],
            )
            .is_err());

        // Neither set — should fail
        assert!(conn
            .execute(
                "INSERT INTO t_positional_args (command_id, subcommand_id, position, name) VALUES (NULL, NULL, 0, 'd')",
                [],
            )
            .is_err());
    }
}
