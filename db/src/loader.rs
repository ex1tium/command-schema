//! Schema database loading with builder pattern and fallback chains.
//!
//! Provides [`SchemaDatabase`] for in-memory schema lookup and
//! [`DatabaseBuilder`] for constructing a database from multiple sources with
//! automatic fallback.
//!
//! # Loading patterns
//!
//! ```no_run
//! use command_schema_db::SchemaDatabase;
//!
//! // Load from a directory of JSON schema files
//! let db = SchemaDatabase::from_dir("schemas/database/").unwrap();
//! assert!(db.get("git").is_some());
//!
//! // Load from a single SchemaPackage JSON bundle
//! let db = SchemaDatabase::from_bundle("schemas.json").unwrap();
//!
//! // Use the builder for a fallback chain
//! let db = SchemaDatabase::builder()
//!     .from_dir("schemas/database/")
//!     .from_bundle("schemas.json")
//!     .build()
//!     .unwrap();
//! ```
//!
//! All lookups are O(1) via the internal `HashMap`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use command_schema_core::{CommandSchema, SchemaPackage};

use crate::error::{DatabaseError, Result};

/// Describes where a [`SchemaDatabase`] was loaded from.
#[derive(Debug, Clone)]
pub enum DatabaseSource {
    /// Loaded from a directory of individual JSON schema files.
    Directory(PathBuf),
    /// Loaded from a single [`SchemaPackage`] JSON file.
    Bundle(PathBuf),
    /// Loaded from build-time embedded compressed schemas.
    Bundled,
    /// Loaded via a fallback chain of multiple sources.
    Multiple(Vec<DatabaseSource>),
}

/// In-memory collection of command schemas with O(1) lookup by name.
///
/// Backed by a [`HashMap`], providing constant-time lookups for any loaded
/// command. Typical startup time is under 100ms for ~200 schemas loaded from
/// a directory.
///
/// # Examples
///
/// ```no_run
/// use command_schema_db::SchemaDatabase;
///
/// // Load from a directory, then query
/// let db = SchemaDatabase::from_dir("schemas/database/").unwrap();
/// println!("Loaded {} schemas", db.len());
///
/// if let Some(schema) = db.get("git") {
///     println!("git has {} subcommands", schema.subcommands.len());
/// }
///
/// // Iterate over all command names
/// for name in db.commands() {
///     println!("  {}", name);
/// }
/// ```
#[derive(Debug)]
pub struct SchemaDatabase {
    schemas: HashMap<String, CommandSchema>,
    source: DatabaseSource,
}

impl SchemaDatabase {
    /// Returns a new [`DatabaseBuilder`] for configuring a fallback chain.
    pub fn builder() -> DatabaseBuilder {
        DatabaseBuilder::new()
    }

    /// Loads schemas from a directory of `*.json` files.
    ///
    /// Each file is parsed as a [`CommandSchema`] and indexed by its `command`
    /// field.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::IoError`] if the directory cannot be read or a
    /// file cannot be opened, or [`DatabaseError::JsonError`] if any file
    /// contains invalid JSON.
    pub fn from_dir(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut schemas = HashMap::new();

        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let file_path = entry.path();
            if file_path.extension().and_then(|e| e.to_str()) == Some("json") {
                let file = std::fs::File::open(&file_path)?;
                let reader = std::io::BufReader::new(file);
                let schema: CommandSchema = serde_json::from_reader(reader)?;
                schemas.insert(schema.command.clone(), schema);
            }
        }

        Ok(Self {
            schemas,
            source: DatabaseSource::Directory(path.to_path_buf()),
        })
    }

    /// Loads schemas from a single [`SchemaPackage`] JSON file.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::IoError`] if the file cannot be read, or
    /// [`DatabaseError::JsonError`] if parsing fails.
    pub fn from_bundle(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let package: SchemaPackage = serde_json::from_reader(reader)?;

        let mut schemas = HashMap::new();
        for schema in package.schemas {
            schemas.insert(schema.command.clone(), schema);
        }

        Ok(Self {
            schemas,
            source: DatabaseSource::Bundle(path.to_path_buf()),
        })
    }

    /// Loads schemas from build-time embedded compressed data.
    ///
    /// Only available when the `bundled-schemas` feature is enabled.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::CompressionError`] if decompression fails, or
    /// [`DatabaseError::JsonError`] if parsing fails.
    #[cfg(feature = "bundled-schemas")]
    pub fn bundled() -> Result<Self> {
        let schemas_vec = crate::bundled::load_bundled_schemas()?;
        if schemas_vec.is_empty() {
            return Err(DatabaseError::NoSourcesAvailable);
        }
        let mut schemas = HashMap::new();
        for schema in schemas_vec {
            schemas.insert(schema.command.clone(), schema);
        }

        Ok(Self {
            schemas,
            source: DatabaseSource::Bundled,
        })
    }

    /// Looks up a schema by command name in O(1) time.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use command_schema_db::SchemaDatabase;
    ///
    /// let db = SchemaDatabase::from_dir("schemas/database/").unwrap();
    /// if let Some(schema) = db.get("git") {
    ///     println!("git: {} global flags", schema.global_flags.len());
    /// }
    /// ```
    pub fn get(&self, command: &str) -> Option<&CommandSchema> {
        self.schemas.get(command)
    }

    /// Looks up a mutable reference to a schema by command name.
    pub fn get_mut(&mut self, command: &str) -> Option<&mut CommandSchema> {
        self.schemas.get_mut(command)
    }

    /// Inserts a schema into the database, replacing any existing entry for
    /// the same command name.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use command_schema_db::SchemaDatabase;
    /// use command_schema_core::{CommandSchema, SchemaSource};
    ///
    /// let mut db = SchemaDatabase::from_dir("schemas/database/").unwrap();
    /// let schema = CommandSchema::new("mycli", SchemaSource::Learned);
    /// db.insert("mycli".into(), schema);
    /// assert!(db.contains("mycli"));
    /// ```
    pub fn insert(&mut self, command: String, schema: CommandSchema) {
        self.schemas.insert(command, schema);
    }

    /// Returns `true` if the database contains a schema for `command`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use command_schema_db::SchemaDatabase;
    ///
    /// let db = SchemaDatabase::from_dir("schemas/database/").unwrap();
    /// if db.contains("docker") {
    ///     println!("Docker schema is available");
    /// }
    /// ```
    pub fn contains(&self, command: &str) -> bool {
        self.schemas.contains_key(command)
    }

    /// Returns the number of schemas in the database.
    pub fn len(&self) -> usize {
        self.schemas.len()
    }

    /// Returns `true` if the database contains no schemas.
    pub fn is_empty(&self) -> bool {
        self.schemas.is_empty()
    }

    /// Returns an iterator over command names.
    pub fn commands(&self) -> impl Iterator<Item = &str> {
        self.schemas.keys().map(|s| s.as_str())
    }

    /// Returns a reference to the source metadata.
    pub fn source(&self) -> &DatabaseSource {
        &self.source
    }
}

/// Builder for constructing a [`SchemaDatabase`] with a fallback chain.
///
/// Sources are tried in the order they are added. The first successful load
/// wins; if all fail, [`DatabaseError::NoSourcesAvailable`] is returned.
///
/// # Example
///
/// ```no_run
/// use command_schema_db::SchemaDatabase;
///
/// let db = SchemaDatabase::builder()
///     .from_dir("/opt/schemas/")
///     .from_bundle("/opt/schemas.json")
///     .build()
///     .unwrap();
/// ```
pub struct DatabaseBuilder {
    sources: Vec<DatabaseSource>,
}

impl DatabaseBuilder {
    /// Creates a new builder with no sources.
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
        }
    }

    /// Adds the embedded bundled schemas as a source.
    ///
    /// Only effective when the `bundled-schemas` feature is enabled; otherwise
    /// this source is silently skipped during [`build`](Self::build).
    pub fn with_bundled(mut self) -> Self {
        self.sources.push(DatabaseSource::Bundled);
        self
    }

    /// Adds a directory of JSON schema files as a source.
    pub fn from_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.sources.push(DatabaseSource::Directory(path.into()));
        self
    }

    /// Adds a [`SchemaPackage`](command_schema_core::SchemaPackage) bundle
    /// file as a source.
    pub fn from_bundle(mut self, path: impl Into<PathBuf>) -> Self {
        self.sources.push(DatabaseSource::Bundle(path.into()));
        self
    }

    /// Attempts to load schemas from configured sources in order.
    ///
    /// Returns the first successfully loaded database. If all sources fail,
    /// returns [`DatabaseError::NoSourcesAvailable`].
    pub fn build(self) -> Result<SchemaDatabase> {
        if self.sources.is_empty() {
            return Err(DatabaseError::NoSourcesAvailable);
        }

        let all_sources = self.sources.clone();

        for source in &self.sources {
            let result = match source {
                DatabaseSource::Directory(path) => SchemaDatabase::from_dir(path),
                DatabaseSource::Bundle(path) => SchemaDatabase::from_bundle(path),
                DatabaseSource::Bundled => {
                    #[cfg(feature = "bundled-schemas")]
                    {
                        SchemaDatabase::bundled()
                    }
                    #[cfg(not(feature = "bundled-schemas"))]
                    {
                        Err(DatabaseError::NoSourcesAvailable)
                    }
                }
                DatabaseSource::Multiple(_) => continue,
            };

            if let Ok(mut db) = result {
                db.source = DatabaseSource::Multiple(all_sources);
                return Ok(db);
            }
        }

        Err(DatabaseError::NoSourcesAvailable)
    }
}

impl Default for DatabaseBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use command_schema_core::{CommandSchema, SchemaSource};
    use std::io::Write;

    fn test_schema(name: &str) -> CommandSchema {
        CommandSchema::new(name, SchemaSource::Bootstrap)
    }

    fn write_schema(dir: &Path, schema: &CommandSchema) {
        let path = dir.join(format!("{}.json", schema.command));
        let mut f = std::fs::File::create(path).unwrap();
        serde_json::to_writer_pretty(&mut f, schema).unwrap();
        f.flush().unwrap();
    }

    #[test]
    fn test_from_dir() {
        let dir = std::env::temp_dir().join("cs_db_test_from_dir");
        std::fs::create_dir_all(&dir).unwrap();

        write_schema(&dir, &test_schema("git"));
        write_schema(&dir, &test_schema("docker"));
        write_schema(&dir, &test_schema("cargo"));

        let db = SchemaDatabase::from_dir(&dir).unwrap();
        assert_eq!(db.len(), 3);
        assert!(db.contains("git"));
        assert!(db.contains("docker"));
        assert!(db.contains("cargo"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_from_bundle() {
        let dir = std::env::temp_dir().join("cs_db_test_from_bundle");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bundle.json");

        let mut package = SchemaPackage::new("1.0.0", "2024-01-01T00:00:00Z");
        package.schemas.push(test_schema("git"));
        package.schemas.push(test_schema("docker"));

        let mut f = std::fs::File::create(&path).unwrap();
        serde_json::to_writer_pretty(&mut f, &package).unwrap();
        f.flush().unwrap();

        let db = SchemaDatabase::from_bundle(&path).unwrap();
        assert_eq!(db.len(), 2);
        assert!(db.contains("git"));
        assert!(db.contains("docker"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_builder_single_source() {
        let dir = std::env::temp_dir().join("cs_db_test_builder_single");
        std::fs::create_dir_all(&dir).unwrap();
        write_schema(&dir, &test_schema("git"));

        let db = SchemaDatabase::builder()
            .from_dir(&dir)
            .build()
            .unwrap();
        assert!(db.contains("git"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_builder_fallback_first_succeeds() {
        let dir = std::env::temp_dir().join("cs_db_test_builder_fb_first");
        std::fs::create_dir_all(&dir).unwrap();
        write_schema(&dir, &test_schema("git"));

        let db = SchemaDatabase::builder()
            .from_dir(&dir)
            .from_bundle("/nonexistent/bundle.json")
            .build()
            .unwrap();
        assert!(db.contains("git"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_builder_fallback_first_fails() {
        let dir = std::env::temp_dir().join("cs_db_test_builder_fb_second");
        std::fs::create_dir_all(&dir).unwrap();
        let bundle_path = dir.join("bundle.json");

        let mut package = SchemaPackage::new("1.0.0", "2024-01-01T00:00:00Z");
        package.schemas.push(test_schema("docker"));

        let mut f = std::fs::File::create(&bundle_path).unwrap();
        serde_json::to_writer_pretty(&mut f, &package).unwrap();
        f.flush().unwrap();

        let db = SchemaDatabase::builder()
            .from_dir("/nonexistent/dir/")
            .from_bundle(&bundle_path)
            .build()
            .unwrap();
        assert!(db.contains("docker"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_builder_all_fail() {
        let result = SchemaDatabase::builder()
            .from_dir("/nonexistent/dir1/")
            .from_bundle("/nonexistent/bundle1.json")
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_hashmap_operations() {
        let dir = std::env::temp_dir().join("cs_db_test_ops");
        std::fs::create_dir_all(&dir).unwrap();
        write_schema(&dir, &test_schema("git"));

        let mut db = SchemaDatabase::from_dir(&dir).unwrap();
        assert!(db.get("git").is_some());
        assert!(db.get("nonexistent").is_none());

        db.insert("cargo".into(), test_schema("cargo"));
        assert_eq!(db.len(), 2);
        assert!(!db.is_empty());
        assert!(db.contains("cargo"));

        assert!(db.get_mut("cargo").is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_commands_iterator() {
        let dir = std::env::temp_dir().join("cs_db_test_iter");
        std::fs::create_dir_all(&dir).unwrap();
        write_schema(&dir, &test_schema("git"));
        write_schema(&dir, &test_schema("docker"));

        let db = SchemaDatabase::from_dir(&dir).unwrap();
        let mut commands: Vec<&str> = db.commands().collect();
        commands.sort();
        assert_eq!(commands, vec!["docker", "git"]);

        std::fs::remove_dir_all(&dir).ok();
    }
}
