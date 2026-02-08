//! Error types for SQLite schema operations.
//!
//! Provides a unified error type covering database access, conversion,
//! migration, and validation failures.

use thiserror::Error;

/// Errors that can occur during SQLite schema operations.
#[derive(Debug, Error)]
pub enum SqliteError {
    /// SQLite database operation failure.
    #[error("database error: {0}")]
    DatabaseError(#[from] rusqlite::Error),

    /// Schema-to-SQL or SQL-to-schema conversion failure.
    #[error("conversion error: {0}")]
    ConversionError(String),

    /// Migration lifecycle operation failure.
    #[error("migration error: {0}")]
    MigrationError(String),

    /// Table prefix contains invalid characters.
    #[error("invalid prefix '{0}': must contain only alphanumeric characters and underscores")]
    InvalidPrefix(String),

    /// Requested command schema was not found.
    #[error("schema not found for command: {0}")]
    SchemaNotFound(String),

    /// Error loading schemas from the static database.
    #[error("loader error: {0}")]
    LoaderError(#[from] command_schema_db::DatabaseError),
}

/// Convenience alias for results with [`SqliteError`].
pub type Result<T> = std::result::Result<T, SqliteError>;
