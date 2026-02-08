//! SQLite storage backend for command schemas.
//!
//! This crate provides a normalized SQLite schema for storing
//! [`CommandSchema`](command_schema_core::CommandSchema) data with full
//! round-trip fidelity. It includes migration lifecycle management,
//! bidirectional conversion between Rust types and SQL rows, and a
//! high-level query interface.
//!
//! # Architecture
//!
//! The crate is organized into four modules:
//!
//! - **`schema`** — SQL generation with customizable table prefixes
//! - **`migration`** — Lifecycle operations (up/down/seed/refresh/status)
//! - **`convert`** — Bidirectional `CommandSchema` ↔ SQL row transformations
//! - **`query`** — Runtime schema access (CRUD operations)
//!
//! # Quick start — migrations
//!
//! ```no_run
//! use command_schema_sqlite::Migration;
//! use rusqlite::Connection;
//!
//! let conn = Connection::open("schemas.db").unwrap();
//! let mut migration = Migration::new(conn, "cs_").unwrap();
//!
//! migration.up().unwrap();
//! migration.seed("schemas/database/").unwrap();
//!
//! let status = migration.status().unwrap();
//! println!("Commands: {}", status.command_count);
//! ```
//!
//! # Quick start — queries
//!
//! ```no_run
//! use command_schema_sqlite::SchemaQuery;
//! use command_schema_core::{CommandSchema, SchemaSource};
//! use rusqlite::Connection;
//!
//! let conn = Connection::open("schemas.db").unwrap();
//! let mut query = SchemaQuery::new(conn, "cs_").unwrap();
//!
//! if let Some(schema) = query.get_schema("git").unwrap() {
//!     println!("{} has {} subcommands", schema.command, schema.subcommands.len());
//! }
//! ```
//!
//! # Table prefix customization
//!
//! All table and index names are prefixed with a configurable string,
//! allowing multiple isolated schema sets within the same SQLite database.
//! Prefixes must contain only alphanumeric characters and underscores.

mod convert;
mod error;
mod migration;
mod query;
mod schema;

pub use error::{Result, SqliteError};
pub use migration::{Migration, MigrationStatus, SeedReport};
pub use query::SchemaQuery;
