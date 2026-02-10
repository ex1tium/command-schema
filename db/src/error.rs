//! Error types for schema database operations.
//!
//! Provides a unified error type covering all failure modes: I/O, serialization,
//! manifest validation, checksum verification, and compression.

use thiserror::Error;

/// Errors that can occur during database operations.
#[derive(Debug, Error)]
pub enum DatabaseError {
    /// File I/O failure.
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// JSON parsing or serialization failure.
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    /// YAML parsing or serialization failure.
    #[error("YAML error: {0}")]
    YamlError(#[from] serde_yaml::Error),

    /// Manifest validation failure (e.g., missing required fields).
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),

    /// Checksum mismatch between expected and actual values.
    #[error("invalid checksum: {0}")]
    InvalidChecksum(String),

    /// All configured loader sources failed.
    #[error("no schema sources available")]
    NoSourcesAvailable,

    /// Gzip compression or decompression failure.
    #[error("compression error: {0}")]
    CompressionError(String),
}

/// Convenience alias for results with [`DatabaseError`].
pub type Result<T> = std::result::Result<T, DatabaseError>;
