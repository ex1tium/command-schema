//! Schema type definitions for command structure modeling.
//!
//! This module defines the core data model used to represent CLI command
//! structures. The types are designed for serialization with [`serde`] and
//! can round-trip through JSON, SQLite, and other storage backends.

use serde::{Deserialize, Serialize};

/// Version of the schema contract (semver).
///
/// Embedded in every [`CommandSchema`] and
/// [`SchemaPackage`](crate::SchemaPackage) to track compatibility across
/// schema versions.
pub const SCHEMA_CONTRACT_VERSION: &str = "1.0.0";

/// Source of schema information.
///
/// Tracks how a schema was obtained, which is useful for prioritizing
/// sources during merges and filtering in queries.
///
/// # Examples
///
/// ```
/// use command_schema_core::SchemaSource;
///
/// let source = SchemaSource::default();
/// assert_eq!(source, SchemaSource::HelpCommand);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SchemaSource {
    /// Extracted from `--help` output (the default source).
    #[default]
    HelpCommand,
    /// Parsed from a man page.
    ManPage,
    /// Manually defined in a bootstrap file.
    Bootstrap,
    /// Learned from user command history.
    Learned,
}

/// Value type for flags and arguments.
///
/// Describes what kind of value a flag or argument accepts. The parser
/// infers these from help text heuristics (e.g., `<FILE>` → `File`,
/// `<N>` → `Number`).
///
/// # Examples
///
/// ```
/// use command_schema_core::ValueType;
///
/// let vt = ValueType::default();
/// assert_eq!(vt, ValueType::Any);
///
/// let choices = ValueType::Choice(vec!["json".into(), "yaml".into()]);
/// assert!(matches!(choices, ValueType::Choice(_)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ValueType {
    /// Boolean flag (no value).
    Bool,
    /// String value.
    String,
    /// Numeric value.
    Number,
    /// File path.
    File,
    /// Directory path.
    Directory,
    /// URL.
    Url,
    /// Git branch name (learned from history).
    Branch,
    /// Git remote name (learned from history).
    Remote,
    /// One of specific choices (e.g., `--format json|yaml|toml`).
    Choice(Vec<String>),
    /// Unknown/any type (the default).
    #[default]
    Any,
}

/// Schema for a command flag.
///
/// A flag has an optional short form (e.g., `-v`) and/or long form
/// (e.g., `--verbose`), an associated value type, and optional metadata
/// like description, multiplicity, and relationships to other flags.
///
/// Use the constructor methods [`boolean`](FlagSchema::boolean) and
/// [`with_value`](FlagSchema::with_value) to create flags, then chain
/// builder methods like [`with_description`](FlagSchema::with_description).
///
/// # Examples
///
/// ```
/// use command_schema_core::{FlagSchema, ValueType};
///
/// // Boolean flag
/// let verbose = FlagSchema::boolean(Some("-v"), Some("--verbose"))
///     .with_description("Enable verbose output");
/// assert_eq!(verbose.canonical_name(), "--verbose");
/// assert!(!verbose.takes_value);
///
/// // Flag that takes a value
/// let output = FlagSchema::with_value(Some("-o"), Some("--output"), ValueType::File);
/// assert!(output.takes_value);
/// assert_eq!(output.value_type, ValueType::File);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagSchema {
    /// Short form (e.g., "-m")
    pub short: Option<String>,
    /// Long form (e.g., "--message")
    pub long: Option<String>,
    /// Type of value this flag accepts
    pub value_type: ValueType,
    /// Whether a value is required
    pub takes_value: bool,
    /// Description from help text
    pub description: Option<String>,
    /// Can this flag appear multiple times?
    pub multiple: bool,
    /// Flags this conflicts with (mutually exclusive)
    pub conflicts_with: Vec<String>,
    /// Flags this requires to also be present
    pub requires: Vec<String>,
}

impl FlagSchema {
    /// Creates a boolean flag (no value).
    ///
    /// # Examples
    ///
    /// ```
    /// use command_schema_core::FlagSchema;
    ///
    /// let flag = FlagSchema::boolean(Some("-v"), Some("--verbose"));
    /// assert!(!flag.takes_value);
    /// assert!(flag.matches("-v"));
    /// assert!(flag.matches("--verbose"));
    /// ```
    pub fn boolean(short: Option<&str>, long: Option<&str>) -> Self {
        Self {
            short: short.map(String::from),
            long: long.map(String::from),
            value_type: ValueType::Bool,
            takes_value: false,
            description: None,
            multiple: false,
            conflicts_with: Vec::new(),
            requires: Vec::new(),
        }
    }

    /// Creates a flag that takes a value.
    ///
    /// # Examples
    ///
    /// ```
    /// use command_schema_core::{FlagSchema, ValueType};
    ///
    /// let flag = FlagSchema::with_value(Some("-m"), Some("--message"), ValueType::String);
    /// assert!(flag.takes_value);
    /// assert_eq!(flag.value_type, ValueType::String);
    /// ```
    pub fn with_value(short: Option<&str>, long: Option<&str>, value_type: ValueType) -> Self {
        Self {
            short: short.map(String::from),
            long: long.map(String::from),
            value_type,
            takes_value: true,
            description: None,
            multiple: false,
            conflicts_with: Vec::new(),
            requires: Vec::new(),
        }
    }

    /// Adds a description.
    pub fn with_description(mut self, desc: &str) -> Self {
        self.description = Some(desc.to_string());
        self
    }

    /// Marks as allowing multiple occurrences.
    pub fn allow_multiple(mut self) -> Self {
        self.multiple = true;
        self
    }

    /// Returns the canonical name (long form preferred, falls back to short).
    ///
    /// # Examples
    ///
    /// ```
    /// use command_schema_core::FlagSchema;
    ///
    /// let flag = FlagSchema::boolean(Some("-v"), Some("--verbose"));
    /// assert_eq!(flag.canonical_name(), "--verbose");
    ///
    /// let short_only = FlagSchema::boolean(Some("-v"), None);
    /// assert_eq!(short_only.canonical_name(), "-v");
    /// ```
    pub fn canonical_name(&self) -> &str {
        self.long
            .as_deref()
            .or(self.short.as_deref())
            .unwrap_or("unknown")
    }

    /// Checks if this flag matches a given string (short or long form).
    ///
    /// # Examples
    ///
    /// ```
    /// use command_schema_core::FlagSchema;
    ///
    /// let flag = FlagSchema::boolean(Some("-v"), Some("--verbose"));
    /// assert!(flag.matches("-v"));
    /// assert!(flag.matches("--verbose"));
    /// assert!(!flag.matches("-x"));
    /// ```
    pub fn matches(&self, s: &str) -> bool {
        self.short.as_deref() == Some(s) || self.long.as_deref() == Some(s)
    }
}

/// Schema for a positional argument.
///
/// Positional arguments are unnamed values that appear after flags in a
/// command invocation (e.g., `cp <SOURCE> <DEST>`).
///
/// # Examples
///
/// ```
/// use command_schema_core::{ArgSchema, ValueType};
///
/// let src = ArgSchema::required("source", ValueType::File);
/// assert!(src.required);
///
/// let dest = ArgSchema::optional("dest", ValueType::Directory)
///     .allow_multiple();
/// assert!(!dest.required);
/// assert!(dest.multiple);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgSchema {
    /// Name of the argument (e.g., "file", "url")
    pub name: String,
    /// Type of value expected
    pub value_type: ValueType,
    /// Is this argument required?
    pub required: bool,
    /// Can multiple values be provided?
    pub multiple: bool,
    /// Description from help text
    pub description: Option<String>,
}

impl ArgSchema {
    /// Creates a required positional argument.
    ///
    /// # Examples
    ///
    /// ```
    /// use command_schema_core::{ArgSchema, ValueType};
    ///
    /// let arg = ArgSchema::required("file", ValueType::File);
    /// assert!(arg.required);
    /// assert_eq!(arg.name, "file");
    /// ```
    pub fn required(name: &str, value_type: ValueType) -> Self {
        Self {
            name: name.to_string(),
            value_type,
            required: true,
            multiple: false,
            description: None,
        }
    }

    /// Creates an optional positional argument.
    ///
    /// # Examples
    ///
    /// ```
    /// use command_schema_core::{ArgSchema, ValueType};
    ///
    /// let arg = ArgSchema::optional("pattern", ValueType::String);
    /// assert!(!arg.required);
    /// ```
    pub fn optional(name: &str, value_type: ValueType) -> Self {
        Self {
            name: name.to_string(),
            value_type,
            required: false,
            multiple: false,
            description: None,
        }
    }

    /// Marks as accepting multiple values.
    pub fn allow_multiple(mut self) -> Self {
        self.multiple = true;
        self
    }
}

/// Schema for a subcommand.
///
/// Subcommands represent nested command hierarchies (e.g., `git remote add`).
/// Each subcommand can have its own flags, positional arguments, aliases, and
/// further nested subcommands.
///
/// # Examples
///
/// ```
/// use command_schema_core::{SubcommandSchema, FlagSchema, ArgSchema, ValueType};
///
/// let sub = SubcommandSchema::new("commit")
///     .with_flag(FlagSchema::with_value(Some("-m"), Some("--message"), ValueType::String))
///     .with_arg(ArgSchema::optional("pathspec", ValueType::File));
///
/// assert_eq!(sub.name, "commit");
/// assert_eq!(sub.flags.len(), 1);
/// assert_eq!(sub.positional.len(), 1);
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubcommandSchema {
    /// Name of the subcommand
    pub name: String,
    /// Short description
    pub description: Option<String>,
    /// Flags specific to this subcommand
    pub flags: Vec<FlagSchema>,
    /// Positional arguments
    pub positional: Vec<ArgSchema>,
    /// Nested subcommands (e.g., git remote add)
    pub subcommands: Vec<SubcommandSchema>,
    /// Aliases for this subcommand
    pub aliases: Vec<String>,
}

impl SubcommandSchema {
    /// Creates a new subcommand schema with the given name.
    ///
    /// # Examples
    ///
    /// ```
    /// use command_schema_core::SubcommandSchema;
    ///
    /// let sub = SubcommandSchema::new("push");
    /// assert_eq!(sub.name, "push");
    /// assert!(sub.flags.is_empty());
    /// ```
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// Adds a flag to this subcommand.
    pub fn with_flag(mut self, flag: FlagSchema) -> Self {
        self.flags.push(flag);
        self
    }

    /// Adds a positional argument.
    pub fn with_arg(mut self, arg: ArgSchema) -> Self {
        self.positional.push(arg);
        self
    }

    /// Adds a nested subcommand.
    pub fn with_subcommand(mut self, sub: SubcommandSchema) -> Self {
        self.subcommands.push(sub);
        self
    }
}

/// Complete schema for a command.
///
/// This is the primary type in the crate. It represents the full structure
/// of a CLI command, including global flags, subcommands, positional args,
/// provenance metadata, and a confidence score.
///
/// # Examples
///
/// ```
/// use command_schema_core::*;
///
/// let mut schema = CommandSchema::new("git", SchemaSource::Bootstrap);
/// schema.description = Some("The stupid content tracker".into());
/// schema.global_flags.push(
///     FlagSchema::boolean(Some("-v"), Some("--verbose")),
/// );
/// schema.subcommands.push(
///     SubcommandSchema::new("commit")
///         .with_flag(FlagSchema::with_value(Some("-m"), Some("--message"), ValueType::String)),
/// );
///
/// assert_eq!(schema.command, "git");
/// assert_eq!(schema.subcommand_names(), vec!["commit"]);
/// assert_eq!(schema.flags_for_subcommand("commit").len(), 2); // global + subcommand
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandSchema {
    /// Schema contract version (populated from [`SCHEMA_CONTRACT_VERSION`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    /// The base command name (e.g., "git", "docker")
    pub command: String,
    /// Short description of the command
    pub description: Option<String>,
    /// Global flags (apply to all subcommands)
    pub global_flags: Vec<FlagSchema>,
    /// Subcommands
    pub subcommands: Vec<SubcommandSchema>,
    /// Positional arguments (for commands without subcommands)
    pub positional: Vec<ArgSchema>,
    /// Where this schema came from
    pub source: SchemaSource,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f64,
    /// Version string if detected
    pub version: Option<String>,
}

impl CommandSchema {
    /// Creates a new command schema with the given name and source.
    ///
    /// The confidence score defaults to `1.0` (maximum).
    ///
    /// # Examples
    ///
    /// ```
    /// use command_schema_core::{CommandSchema, SchemaSource};
    ///
    /// let schema = CommandSchema::new("docker", SchemaSource::HelpCommand);
    /// assert_eq!(schema.command, "docker");
    /// assert_eq!(schema.confidence, 1.0);
    /// ```
    pub fn new(command: &str, source: SchemaSource) -> Self {
        Self {
            command: command.to_string(),
            source,
            confidence: 1.0,
            ..Default::default()
        }
    }

    /// Finds a subcommand by name or alias.
    ///
    /// # Examples
    ///
    /// ```
    /// use command_schema_core::{CommandSchema, SchemaSource, SubcommandSchema};
    ///
    /// let mut schema = CommandSchema::new("git", SchemaSource::Bootstrap);
    /// schema.subcommands.push(SubcommandSchema::new("commit"));
    ///
    /// assert!(schema.find_subcommand("commit").is_some());
    /// assert!(schema.find_subcommand("nonexistent").is_none());
    /// ```
    pub fn find_subcommand(&self, name: &str) -> Option<&SubcommandSchema> {
        self.subcommands
            .iter()
            .find(|s| s.name == name || s.aliases.contains(&name.to_string()))
    }

    /// Finds a global flag by short or long form.
    ///
    /// # Examples
    ///
    /// ```
    /// use command_schema_core::{CommandSchema, SchemaSource, FlagSchema};
    ///
    /// let mut schema = CommandSchema::new("git", SchemaSource::Bootstrap);
    /// schema.global_flags.push(FlagSchema::boolean(Some("-v"), Some("--verbose")));
    ///
    /// assert!(schema.find_global_flag("--verbose").is_some());
    /// assert!(schema.find_global_flag("-v").is_some());
    /// assert!(schema.find_global_flag("--debug").is_none());
    /// ```
    pub fn find_global_flag(&self, flag: &str) -> Option<&FlagSchema> {
        self.global_flags.iter().find(|f| f.matches(flag))
    }

    /// Gets all subcommand names.
    pub fn subcommand_names(&self) -> Vec<&str> {
        self.subcommands.iter().map(|s| s.name.as_str()).collect()
    }

    /// Gets all flags for a specific subcommand (global + subcommand-specific).
    ///
    /// Returns global flags first, followed by subcommand-specific flags.
    ///
    /// # Examples
    ///
    /// ```
    /// use command_schema_core::*;
    ///
    /// let mut schema = CommandSchema::new("git", SchemaSource::Bootstrap);
    /// schema.global_flags.push(FlagSchema::boolean(Some("-v"), Some("--verbose")));
    /// schema.subcommands.push(
    ///     SubcommandSchema::new("commit")
    ///         .with_flag(FlagSchema::with_value(Some("-m"), Some("--message"), ValueType::String)),
    /// );
    ///
    /// let flags = schema.flags_for_subcommand("commit");
    /// assert_eq!(flags.len(), 2); // --verbose (global) + --message (subcommand)
    /// ```
    pub fn flags_for_subcommand(&self, subcommand: &str) -> Vec<&FlagSchema> {
        let mut flags: Vec<&FlagSchema> = self.global_flags.iter().collect();
        if let Some(sub) = self.find_subcommand(subcommand) {
            flags.extend(sub.flags.iter());
        }
        flags
    }
}

/// Result of schema extraction attempt.
///
/// Returned by the discovery crate's `parse_help_text()` function. Contains
/// the extracted schema (if successful), the raw help output, detected format,
/// and any warnings encountered during parsing.
///
/// # Examples
///
/// ```
/// use command_schema_core::ExtractionResult;
///
/// // A successful extraction
/// let result = ExtractionResult {
///     schema: None,
///     raw_output: String::new(),
///     detected_format: None,
///     warnings: vec!["No subcommands found".into()],
///     success: false,
/// };
/// assert!(!result.success);
/// ```
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    /// The extracted schema (if successful)
    pub schema: Option<CommandSchema>,
    /// Raw help output that was parsed
    pub raw_output: String,
    /// Format that was detected
    pub detected_format: Option<HelpFormat>,
    /// Warnings encountered during parsing
    pub warnings: Vec<String>,
    /// Whether extraction was successful
    pub success: bool,
}

/// Detected help output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpFormat {
    /// Rust Clap library format
    Clap,
    /// Go Cobra library format
    Cobra,
    /// Python argparse format
    Argparse,
    /// Docopt format
    Docopt,
    /// GNU standard format
    Gnu,
    /// BSD style
    Bsd,
    /// Unknown/custom format
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flag_schema_creation() {
        let flag = FlagSchema::boolean(Some("-v"), Some("--verbose"))
            .with_description("Enable verbose output");

        assert_eq!(flag.short, Some("-v".to_string()));
        assert_eq!(flag.long, Some("--verbose".to_string()));
        assert!(!flag.takes_value);
        assert_eq!(flag.canonical_name(), "--verbose");
    }

    #[test]
    fn test_flag_with_value() {
        let flag = FlagSchema::with_value(Some("-m"), Some("--message"), ValueType::String);

        assert!(flag.takes_value);
        assert_eq!(flag.value_type, ValueType::String);
    }

    #[test]
    fn test_flag_matches() {
        let flag = FlagSchema::boolean(Some("-v"), Some("--verbose"));

        assert!(flag.matches("-v"));
        assert!(flag.matches("--verbose"));
        assert!(!flag.matches("-x"));
    }

    #[test]
    fn test_command_schema_find_subcommand() {
        let mut schema = CommandSchema::new("git", SchemaSource::Bootstrap);
        schema.subcommands.push(SubcommandSchema::new("commit"));
        schema.subcommands.push(SubcommandSchema::new("push"));

        assert!(schema.find_subcommand("commit").is_some());
        assert!(schema.find_subcommand("pull").is_none());
    }
}
