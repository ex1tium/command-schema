//! Migration lifecycle operations for the SQLite schema.
//!
//! Provides [`Migration`] for creating, dropping, seeding, and refreshing
//! the normalized table structure. All mutation operations use transactions
//! to ensure atomicity.
//!
//! # Example
//!
//! ```no_run
//! use command_schema_sqlite::Migration;
//! use rusqlite::Connection;
//!
//! let conn = Connection::open("schemas.db").unwrap();
//! let mut migration = Migration::new(conn, "cs_").unwrap();
//!
//! // Create tables
//! migration.up().unwrap();
//!
//! // Check status
//! let status = migration.status().unwrap();
//! assert!(status.tables_exist);
//!
//! // Seed from a directory of JSON schema files
//! migration.seed("schemas/database/").unwrap();
//!
//! // Drop and recreate
//! migration.refresh("schemas/database/").unwrap();
//! ```

use std::collections::HashMap;
use std::path::Path;

use command_schema_db::SchemaDatabase;
use rusqlite::Connection;

use crate::convert::{self, InsertCounts};
use crate::error::{Result, SqliteError};
use crate::schema::{generate_drop_sql, generate_schema_sql, validate_prefix};

/// Manages the lifecycle of the SQLite schema tables.
///
/// Provides operations to create tables ([`up`](Self::up)), drop them
/// ([`down`](Self::down)), seed data from JSON files ([`seed`](Self::seed)),
/// and check the current migration status ([`status`](Self::status)).
///
/// All mutation operations use transactions to ensure atomicity — either
/// all changes succeed or none are applied.
///
/// # Examples
///
/// ```no_run
/// use command_schema_sqlite::Migration;
/// use rusqlite::Connection;
///
/// let conn = Connection::open("schemas.db").unwrap();
/// let mut migration = Migration::new(conn, "cs_").unwrap();
///
/// // Create tables
/// migration.up().unwrap();
///
/// // Seed from JSON files
/// let report = migration.seed("schemas/database/").unwrap();
/// println!("Inserted {} commands, {} flags",
///     report.commands_inserted, report.flags_inserted);
///
/// // Check status
/// let status = migration.status().unwrap();
/// println!("Tables exist: {}, Commands: {}",
///     status.tables_exist, status.command_count);
///
/// // Full reset (drop + recreate + seed)
/// migration.refresh("schemas/database/").unwrap();
/// ```
pub struct Migration {
    conn: Connection,
    prefix: String,
}

impl Migration {
    /// Creates a new migration manager for the given connection and table prefix.
    ///
    /// # Errors
    ///
    /// Returns [`SqliteError::InvalidPrefix`] if the prefix contains invalid characters.
    pub fn new(conn: Connection, prefix: impl Into<String>) -> Result<Self> {
        let prefix = prefix.into();
        validate_prefix(&prefix)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { conn, prefix })
    }

    /// Creates all schema tables and indexes.
    ///
    /// Uses `CREATE TABLE IF NOT EXISTS` so it is safe to call multiple times.
    /// Executes within a transaction for atomicity.
    pub fn up(&mut self) -> Result<()> {
        let sql = generate_schema_sql(&self.prefix)?;
        let tx = self.conn.transaction()?;
        tx.execute_batch(&sql)
            .map_err(|e| SqliteError::MigrationError(format!("failed to create tables: {e}")))?;
        tx.commit()?;
        Ok(())
    }

    /// Drops all schema tables in reverse dependency order.
    ///
    /// Uses `DROP TABLE IF EXISTS` so it is safe to call even if tables
    /// do not exist. Executes within a transaction for atomicity.
    pub fn down(&mut self) -> Result<()> {
        let sql = generate_drop_sql(&self.prefix)?;
        let tx = self.conn.transaction()?;
        tx.execute_batch(&sql)
            .map_err(|e| SqliteError::MigrationError(format!("failed to drop tables: {e}")))?;
        tx.commit()?;
        Ok(())
    }

    /// Returns the current status of the migration.
    ///
    /// Checks whether tables exist and reports row counts for each
    /// primary table.
    pub fn status(&self) -> Result<MigrationStatus> {
        let tables_exist = self.tables_exist()?;

        if !tables_exist {
            return Ok(MigrationStatus {
                tables_exist: false,
                command_count: 0,
                flag_count: 0,
                subcommand_count: 0,
                arg_count: 0,
            });
        }

        let command_count = self.count_rows("commands")?;
        let flag_count = self.count_rows("flags")?;
        let subcommand_count = self.count_rows("subcommands")?;
        let arg_count = self.count_rows("positional_args")?;

        Ok(MigrationStatus {
            tables_exist,
            command_count,
            flag_count,
            subcommand_count,
            arg_count,
        })
    }

    /// Seeds the database from a directory of JSON schema files.
    ///
    /// Loads schemas using [`SchemaDatabase::from_dir`], then inserts each
    /// schema into the SQLite tables within a single transaction.
    ///
    /// # Errors
    ///
    /// Returns [`SqliteError::LoaderError`] if the directory cannot be read,
    /// or [`SqliteError::DatabaseError`] if insertion fails.
    pub fn seed(&mut self, source_dir: impl AsRef<Path>) -> Result<SeedReport> {
        let db = SchemaDatabase::from_dir(source_dir)?;
        let commands: Vec<_> = db.commands().map(String::from).collect();

        let tx = self.conn.transaction()?;
        let mut report = SeedReport::default();

        for cmd_name in &commands {
            let schema = db.get(cmd_name).unwrap();
            let command_id = convert::insert_command(&tx, &self.prefix, schema)?;
            report.commands_inserted += 1;

            // Insert global flags first; the returned map enables cross-scope
            // relationship resolution for subcommand flags.
            let empty = HashMap::new();
            let (flag_counts, global_flag_ids) =
                convert::insert_flags(&tx, &self.prefix, command_id, None, &schema.global_flags, &empty)?;
            report.merge_counts(&flag_counts);

            // Insert global positional args
            let arg_counts = convert::insert_positional_args(
                &tx,
                &self.prefix,
                Some(command_id),
                None,
                &schema.positional,
            )?;
            report.merge_counts(&arg_counts);

            // Insert subcommands (recursive, with global flag map for cross-scope relationships)
            let sub_counts = convert::insert_subcommands(
                &tx,
                &self.prefix,
                command_id,
                None,
                &schema.subcommands,
                &global_flag_ids,
            )?;
            report.merge_counts(&sub_counts);
        }

        tx.commit()?;
        Ok(report)
    }

    /// Drops all tables, recreates them, and seeds from the given directory.
    ///
    /// Equivalent to calling [`down`](Self::down), [`up`](Self::up), then
    /// [`seed`](Self::seed) in sequence.
    pub fn refresh(&mut self, source_dir: impl AsRef<Path>) -> Result<SeedReport> {
        self.down()?;
        self.up()?;
        self.seed(source_dir)
    }

    /// Returns a reference to the underlying connection.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Consumes the migration and returns the underlying connection.
    pub fn into_connection(self) -> Connection {
        self.conn
    }

    /// Checks whether the commands table exists.
    fn tables_exist(&self) -> Result<bool> {
        let table_name = format!("{}commands", self.prefix);
        let mut stmt = self.conn.prepare(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        )?;
        let count: i64 = stmt.query_row([&table_name], |row| row.get(0))?;
        Ok(count > 0)
    }

    /// Counts rows in a prefixed table.
    fn count_rows(&self, table: &str) -> Result<usize> {
        let full_table = format!("{}{}", self.prefix, table);
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT COUNT(*) FROM {full_table}"))?;
        let count: i64 = stmt.query_row([], |row| row.get(0))?;
        Ok(count as usize)
    }
}

/// Status of the current migration state.
///
/// Returned by [`Migration::status`], providing a snapshot of whether
/// tables exist and how many rows are in each primary table.
#[derive(Debug, Clone)]
pub struct MigrationStatus {
    /// Whether the schema tables exist in the database.
    pub tables_exist: bool,
    /// Number of commands stored.
    pub command_count: usize,
    /// Number of flags stored.
    pub flag_count: usize,
    /// Number of subcommands stored.
    pub subcommand_count: usize,
    /// Number of positional arguments stored.
    pub arg_count: usize,
}

/// Report of a seed operation, tracking how many items were inserted.
///
/// Returned by [`Migration::seed`] and [`Migration::refresh`], providing
/// a breakdown of how many commands, flags, subcommands, args, choices,
/// aliases, and relationships were inserted.
#[derive(Debug, Clone, Default)]
pub struct SeedReport {
    /// Number of commands inserted.
    pub commands_inserted: usize,
    /// Number of flags inserted.
    pub flags_inserted: usize,
    /// Number of subcommands inserted.
    pub subcommands_inserted: usize,
    /// Number of positional arguments inserted.
    pub args_inserted: usize,
    /// Number of choice values inserted.
    pub choices_inserted: usize,
    /// Number of aliases inserted.
    pub aliases_inserted: usize,
    /// Number of flag relationships inserted.
    pub relationships_inserted: usize,
}

impl SeedReport {
    /// Merges insert counts from a conversion operation into this report.
    fn merge_counts(&mut self, counts: &InsertCounts) {
        self.flags_inserted += counts.flags;
        self.subcommands_inserted += counts.subcommands;
        self.args_inserted += counts.args;
        self.choices_inserted += counts.choices;
        self.aliases_inserted += counts.aliases;
        self.relationships_inserted += counts.relationships;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migration_new_validates_prefix() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(Migration::new(conn, "valid_prefix_").is_ok());

        let conn = Connection::open_in_memory().unwrap();
        assert!(Migration::new(conn, "").is_err());

        let conn = Connection::open_in_memory().unwrap();
        assert!(Migration::new(conn, "drop;--").is_err());
    }

    #[test]
    fn test_status_on_empty_database() {
        let conn = Connection::open_in_memory().unwrap();
        let migration = Migration::new(conn, "cs_").unwrap();
        let status = migration.status().unwrap();
        assert!(!status.tables_exist);
        assert_eq!(status.command_count, 0);
    }

    #[test]
    fn test_up_and_status() {
        let conn = Connection::open_in_memory().unwrap();
        let mut migration = Migration::new(conn, "cs_").unwrap();
        migration.up().unwrap();
        let status = migration.status().unwrap();
        assert!(status.tables_exist);
        assert_eq!(status.command_count, 0);
        assert_eq!(status.flag_count, 0);
    }

    #[test]
    fn test_up_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        let mut migration = Migration::new(conn, "cs_").unwrap();
        migration.up().unwrap();
        migration.up().unwrap(); // Should not fail
        assert!(migration.status().unwrap().tables_exist);
    }

    #[test]
    fn test_down_removes_tables() {
        let conn = Connection::open_in_memory().unwrap();
        let mut migration = Migration::new(conn, "cs_").unwrap();
        migration.up().unwrap();
        assert!(migration.status().unwrap().tables_exist);

        migration.down().unwrap();
        assert!(!migration.status().unwrap().tables_exist);
    }

    #[test]
    fn test_down_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        let mut migration = Migration::new(conn, "cs_").unwrap();
        migration.down().unwrap(); // No tables to drop, should be fine
    }
}
