//! Static database loading and manifest management for command schemas.
//!
//! This crate provides infrastructure for loading pre-extracted command schemas
//! from various sources (directories, bundles, embedded data) and managing
//! version metadata through manifests.
//!
//! # Quick start
//!
//! ```no_run
//! use command_schema_db::{SchemaDatabase, Manifest, QualityPolicyFingerprint};
//!
//! // Load schemas from a directory
//! let db = SchemaDatabase::from_dir("schemas/database/").unwrap();
//! if let Some(schema) = db.get("git") {
//!     println!("git has {} subcommands", schema.subcommands.len());
//! }
//!
//! // Use the builder for fallback chains
//! let db = SchemaDatabase::builder()
//!     .from_dir("schemas/database/")
//!     .from_bundle("schemas.json")
//!     .build()
//!     .unwrap();
//!
//! // Track extraction state with a manifest
//! let manifest = Manifest::new(
//!     "0.1.0".into(),
//!     QualityPolicyFingerprint::default(),
//! );
//! ```
//!
//! # Feature flags
//!
//! - **`bundled-schemas`**: Enables build-time compression and runtime
//!   decompression of embedded schemas via the `bundled` module.
//!   Requires the `flate2` dependency.

mod config;
mod error;
mod loader;
mod manifest;

#[cfg(feature = "bundled-schemas")]
mod bundled {
    include!(concat!(env!("OUT_DIR"), "/bundled.rs"));
}

pub use config::{CIConfig, ExtractionConfig, QualityConfig};
pub use error::{DatabaseError, Result};
pub use loader::{DatabaseBuilder, DatabaseSource, SchemaDatabase};
pub use manifest::{CommandMetadata, Manifest, QualityPolicyFingerprint};

#[cfg(feature = "bundled-schemas")]
pub use bundled::load_bundled_schemas;
