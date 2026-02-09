use serde::{Deserialize, Serialize};

use crate::CommandSchema;

/// Serializable schema bundle used for curation and distribution.
///
/// A package groups multiple [`CommandSchema`] values with version metadata,
/// making it suitable for serializing to JSON and distributing as a single
/// bundle file or embedding at build time.
///
/// # Examples
///
/// ```
/// use command_schema_core::*;
///
/// let mut package = SchemaPackage::new("1.0.0", "2024-01-15T10:30:00Z");
/// package.name = Some("my-schemas".into());
/// package.schemas.push(CommandSchema::new("git", SchemaSource::Bootstrap));
/// package.schemas.push(CommandSchema::new("docker", SchemaSource::HelpCommand));
///
/// assert_eq!(package.schema_count(), 2);
/// assert_eq!(package.version, "1.0.0");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaPackage {
    /// Schema contract version (populated from
    /// [`SCHEMA_CONTRACT_VERSION`](crate::SCHEMA_CONTRACT_VERSION)).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    /// Package format version (semver string).
    pub version: String,
    /// Optional package name.
    pub name: Option<String>,
    /// Optional package description.
    pub description: Option<String>,
    /// ISO-8601 timestamp for package creation.
    pub generated_at: String,
    /// Optional hash of deterministic bundle content.
    pub bundle_hash: Option<String>,
    /// Command schemas included in this package.
    pub schemas: Vec<CommandSchema>,
}

impl SchemaPackage {
    /// Creates a package with required fields.
    ///
    /// The `schema_version` is automatically set from
    /// [`SCHEMA_CONTRACT_VERSION`](crate::SCHEMA_CONTRACT_VERSION).
    ///
    /// # Examples
    ///
    /// ```
    /// use command_schema_core::SchemaPackage;
    ///
    /// let package = SchemaPackage::new("1.0.0", "2024-01-15T10:30:00Z");
    /// assert_eq!(package.version, "1.0.0");
    /// assert_eq!(package.schema_count(), 0);
    /// ```
    pub fn new(version: impl Into<String>, generated_at: impl Into<String>) -> Self {
        Self {
            schema_version: Some(crate::SCHEMA_CONTRACT_VERSION.to_string()),
            version: version.into(),
            name: None,
            description: None,
            generated_at: generated_at.into(),
            bundle_hash: None,
            schemas: Vec::new(),
        }
    }

    /// Returns the number of schemas in this package.
    pub fn schema_count(&self) -> usize {
        self.schemas.len()
    }
}
