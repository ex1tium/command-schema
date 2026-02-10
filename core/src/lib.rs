//! Core schema types and shared schema package primitives.
//!
//! This crate defines the foundational types for modeling CLI command
//! structures:
//!
//! - [`CommandSchema`] — top-level schema for a command (flags, subcommands,
//!   positional args, metadata).
//! - [`FlagSchema`] — a flag/option with short/long forms, value types, and
//!   relationships.
//! - [`ArgSchema`] — a positional argument with type and multiplicity.
//! - [`SubcommandSchema`] — a subcommand with its own flags, args, and nested
//!   subcommands.
//! - [`SchemaPackage`] — a versioned bundle of command schemas for
//!   distribution.
//!
//! Validation ([`validate_schema`], [`validate_package`]) catches structural
//! errors such as duplicate flags, invalid flag formats, and subcommand cycles.
//!
//! Merging ([`merge_schemas`]) combines two schemas using a configurable
//! [`MergeStrategy`].
//!
//! # Example
//!
//! ```
//! use command_schema_core::*;
//!
//! // Build a schema for a fictional CLI
//! let mut schema = CommandSchema::new("mycli", SchemaSource::Bootstrap);
//! schema.global_flags.push(
//!     FlagSchema::boolean(Some("-v"), Some("--verbose"))
//!         .with_description("Enable verbose output"),
//! );
//! schema.subcommands.push(
//!     SubcommandSchema::new("run")
//!         .with_flag(FlagSchema::with_value(None, Some("--port"), ValueType::Number))
//!         .with_arg(ArgSchema::required("script", ValueType::File)),
//! );
//!
//! assert_eq!(schema.find_subcommand("run").unwrap().name, "run");
//! assert!(schema.find_global_flag("--verbose").is_some());
//! assert!(validate_schema(&schema).is_empty());
//! ```

mod merge;
mod package;
mod types;
mod validate;

pub use merge::{MergeStrategy, merge_schemas};
pub use package::SchemaPackage;
pub use types::*;
pub use validate::{ValidationError, validate_package, validate_schema};
