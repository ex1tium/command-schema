//! Runtime schema access via SQLite queries.
//!
//! Provides [`SchemaQuery`] for CRUD operations on command schemas stored
//! in the normalized SQLite tables. All mutations use transactions to
//! ensure atomicity, and the cascading foreign key constraints handle
//! cleanup automatically on updates and deletes.
//!
//! # Example
//!
//! ```no_run
//! use command_schema_sqlite::SchemaQuery;
//! use command_schema_core::{CommandSchema, SchemaSource};
//! use rusqlite::Connection;
//!
//! let conn = Connection::open("schemas.db").unwrap();
//! let query = SchemaQuery::new(&conn, "cs_").unwrap();
//!
//! // Insert a schema
//! let schema = CommandSchema::new("git", SchemaSource::Bootstrap);
//! query.insert_schema(&schema).unwrap();
//!
//! // Retrieve it
//! let loaded = query.get_schema("git").unwrap();
//! assert!(loaded.is_some());
//!
//! // Delete it
//! query.delete_schema("git").unwrap();
//! ```

use std::collections::HashMap;

use command_schema_core::{CommandSchema, SchemaSource};
use rusqlite::{Connection, params};

use crate::convert;
use crate::error::{Result, SqliteError};
use crate::schema::validate_prefix;

/// Query interface for reading and writing command schemas in SQLite.
///
/// Wraps a connection and table prefix, providing high-level CRUD
/// operations that delegate to the `convert` module for the actual
/// row-level transformations. All mutations use transactions to ensure
/// atomicity.
///
/// # Examples
///
/// ```no_run
/// use command_schema_sqlite::SchemaQuery;
/// use command_schema_core::{CommandSchema, SchemaSource};
/// use rusqlite::Connection;
///
/// let conn = Connection::open("schemas.db").unwrap();
/// let query = SchemaQuery::new(&conn, "cs_").unwrap();
///
/// // Insert a schema
/// let schema = CommandSchema::new("git", SchemaSource::Bootstrap);
/// query.insert_schema(&schema).unwrap();
///
/// // Look it up
/// let loaded = query.get_schema("git").unwrap();
/// assert!(loaded.is_some());
///
/// // List all schemas
/// let all = query.get_all_schemas().unwrap();
/// println!("Total schemas: {}", all.len());
///
/// // Filter by source
/// let learned = query.get_by_source(SchemaSource::Learned).unwrap();
/// println!("Learned schemas: {}", learned.len());
/// ```
pub struct SchemaQuery<'a> {
    conn: &'a Connection,
    prefix: String,
}

impl<'a> SchemaQuery<'a> {
    /// Creates a new query interface for the given connection and table prefix.
    ///
    /// # Errors
    ///
    /// Returns [`SqliteError::InvalidPrefix`] if the prefix is invalid.
    pub fn new(conn: &'a Connection, prefix: impl Into<String>) -> Result<Self> {
        let prefix = prefix.into();
        validate_prefix(&prefix)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { conn, prefix })
    }

    /// Loads a single command schema by name.
    ///
    /// Returns `None` if no command with the given name exists in the database.
    /// The returned schema includes all flags, subcommands (recursive),
    /// positional args, choices, aliases, and flag relationships.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use command_schema_sqlite::SchemaQuery;
    /// # use rusqlite::Connection;
    /// # let conn = Connection::open("schemas.db").unwrap();
    /// # let query = SchemaQuery::new(&conn, "cs_").unwrap();
    /// match query.get_schema("docker").unwrap() {
    ///     Some(schema) => println!("{} subcommands", schema.subcommands.len()),
    ///     None => println!("docker schema not found"),
    /// }
    /// ```
    pub fn get_schema(&self, command: &str) -> Result<Option<CommandSchema>> {
        convert::load_command(self.conn, &self.prefix, command)
    }

    /// Loads all command schemas from the database.
    ///
    /// Queries all command names first, then loads each schema individually.
    /// Returns an empty vector if no commands exist.
    pub fn get_all_schemas(&self) -> Result<Vec<CommandSchema>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT name FROM {}commands ORDER BY name",
            self.prefix
        ))?;

        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut schemas = Vec::with_capacity(names.len());
        for name in &names {
            if let Some(schema) = self.get_schema(name)? {
                schemas.push(schema);
            }
        }
        Ok(schemas)
    }

    /// Loads command schemas filtered by their [`SchemaSource`].
    ///
    /// Returns all schemas that were created from the specified source.
    /// Useful for separating static/bootstrap schemas from learned ones.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use command_schema_sqlite::SchemaQuery;
    /// # use command_schema_core::SchemaSource;
    /// # use rusqlite::Connection;
    /// # let conn = Connection::open("schemas.db").unwrap();
    /// # let query = SchemaQuery::new(&conn, "cs_").unwrap();
    /// let learned = query.get_by_source(SchemaSource::Learned).unwrap();
    /// for schema in &learned {
    ///     println!("Learned: {}", schema.command);
    /// }
    /// ```
    pub fn get_by_source(&self, source: SchemaSource) -> Result<Vec<CommandSchema>> {
        let source_str = convert::source_to_string(&source);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT name FROM {}commands WHERE source = ?1 ORDER BY name",
            self.prefix
        ))?;

        let names: Vec<String> = stmt
            .query_map(params![source_str], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut schemas = Vec::with_capacity(names.len());
        for name in &names {
            if let Some(schema) = self.get_schema(name)? {
                schemas.push(schema);
            }
        }
        Ok(schemas)
    }

    /// Inserts a new command schema into the database.
    ///
    /// All related data (flags, subcommands, args, choices, aliases,
    /// relationships) is inserted within a single transaction.
    ///
    /// # Errors
    ///
    /// Returns an error if a command with the same name already exists.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use command_schema_sqlite::SchemaQuery;
    /// # use command_schema_core::{CommandSchema, SchemaSource, FlagSchema};
    /// # use rusqlite::Connection;
    /// # let conn = Connection::open("schemas.db").unwrap();
    /// # let query = SchemaQuery::new(&conn, "cs_").unwrap();
    /// let mut schema = CommandSchema::new("mycli", SchemaSource::Learned);
    /// schema.global_flags.push(FlagSchema::boolean(Some("-v"), Some("--verbose")));
    /// query.insert_schema(&schema).unwrap();
    /// ```
    pub fn insert_schema(&self, schema: &CommandSchema) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        let command_id = convert::insert_command(&tx, &self.prefix, schema)?;

        // Insert global flags first; the returned map enables cross-scope
        // relationship resolution for subcommand flags.
        let empty = HashMap::new();
        let (_flag_counts, global_flag_ids) = convert::insert_flags(
            &tx,
            &self.prefix,
            command_id,
            None,
            &schema.global_flags,
            &empty,
        )?;

        convert::insert_positional_args(
            &tx,
            &self.prefix,
            Some(command_id),
            None,
            &schema.positional,
        )?;

        convert::insert_subcommands(
            &tx,
            &self.prefix,
            command_id,
            None,
            &schema.subcommands,
            &global_flag_ids,
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Updates an existing command schema by replacing it entirely.
    ///
    /// Deletes the existing command (cascading to all related rows) and
    /// inserts the new schema. Operates within a single transaction.
    ///
    /// # Errors
    ///
    /// Returns [`SqliteError::SchemaNotFound`] if no command with the
    /// schema's name exists.
    pub fn update_schema(&self, schema: &CommandSchema) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        // Check existence
        let exists: bool = tx
            .prepare(&format!(
                "SELECT COUNT(*) FROM {}commands WHERE name = ?1",
                self.prefix
            ))?
            .query_row(params![schema.command], |row| Ok(row.get::<_, i64>(0)? > 0))?;

        if !exists {
            return Err(SqliteError::SchemaNotFound(schema.command.clone()));
        }

        // Delete existing (CASCADE handles cleanup)
        tx.execute(
            &format!("DELETE FROM {}commands WHERE name = ?1", self.prefix),
            params![schema.command],
        )?;

        // Insert fresh
        let command_id = convert::insert_command(&tx, &self.prefix, schema)?;

        let empty = HashMap::new();
        let (_flag_counts, global_flag_ids) = convert::insert_flags(
            &tx,
            &self.prefix,
            command_id,
            None,
            &schema.global_flags,
            &empty,
        )?;

        convert::insert_positional_args(
            &tx,
            &self.prefix,
            Some(command_id),
            None,
            &schema.positional,
        )?;

        convert::insert_subcommands(
            &tx,
            &self.prefix,
            command_id,
            None,
            &schema.subcommands,
            &global_flag_ids,
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Deletes a command schema and all related data.
    ///
    /// The cascading foreign key constraints ensure that all flags,
    /// subcommands, args, choices, aliases, and relationships are
    /// removed automatically.
    ///
    /// # Errors
    ///
    /// Returns [`SqliteError::SchemaNotFound`] if no command with the
    /// given name exists.
    pub fn delete_schema(&self, command: &str) -> Result<()> {
        let rows = self.conn.execute(
            &format!("DELETE FROM {}commands WHERE name = ?1", self.prefix),
            params![command],
        )?;

        if rows == 0 {
            return Err(SqliteError::SchemaNotFound(command.to_string()));
        }

        Ok(())
    }

    /// Returns a reference to the underlying connection.
    pub fn connection(&self) -> &Connection {
        self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_query_validates_prefix() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(SchemaQuery::new(&conn, "valid_").is_ok());

        let conn = Connection::open_in_memory().unwrap();
        assert!(SchemaQuery::new(&conn, "").is_err());
    }
}
