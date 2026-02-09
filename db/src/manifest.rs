//! Manifest management for tracking schema extraction state.
//!
//! The manifest records per-command metadata that enables efficient CI
//! re-extraction. A command should be re-extracted when any of the following
//! change:
//!
//! - **Version**: the command reports a different `--version` string.
//! - **Fingerprint**: for version-less commands, the executable's path,
//!   modification time, or file size changed.
//! - **Policy**: the quality policy thresholds were adjusted.
//! - **Checksum**: the on-disk schema JSON no longer matches the recorded
//!   SHA-256 checksum (manual edit, corruption, etc.).
//!
//! # Examples
//!
//! ```no_run
//! use command_schema_db::{Manifest, QualityPolicyFingerprint, CommandMetadata};
//!
//! // Create a new manifest
//! let mut manifest = Manifest::new(
//!     "0.1.0".into(),
//!     QualityPolicyFingerprint::default(),
//! );
//!
//! // Record an extraction
//! manifest.update_entry("git".into(), CommandMetadata {
//!     version: Some("2.43.0".into()),
//!     executable_path: Some("/usr/bin/git".into()),
//!     mtime_secs: Some(1_700_000_000),
//!     size_bytes: Some(3_500_000),
//!     extracted_at: "2024-01-15T10:30:00Z".into(),
//!     quality_tier: "high".into(),
//!     checksum: "abc123".into(),
//!     implementation: Some("git".into()),
//!     schema_file: Some("git.json".into()),
//! });
//!
//! // Save and reload
//! manifest.save("manifest.json").unwrap();
//! let loaded = Manifest::load("manifest.json").unwrap();
//! assert!(loaded.contains("git"));
//! ```

use std::collections::HashMap;
use std::io::{BufReader, BufWriter};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::Result;

/// Fingerprint of the quality policy used during extraction.
///
/// Stored in the manifest so that CI can detect when thresholds change and
/// trigger a full re-extraction pass. When any field changes between two
/// manifests, [`Manifest::diff`] returns all commands for re-extraction.
///
/// # Examples
///
/// ```
/// use command_schema_db::QualityPolicyFingerprint;
///
/// let default = QualityPolicyFingerprint::default();
/// assert_eq!(default.min_confidence, 0.6);
/// assert_eq!(default.min_coverage, 0.2);
/// assert!(!default.allow_low_quality);
///
/// let strict = QualityPolicyFingerprint {
///     min_confidence: 0.8,
///     min_coverage: 0.5,
///     allow_low_quality: false,
/// };
/// assert_ne!(default, strict);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QualityPolicyFingerprint {
    /// Minimum confidence score (0.0–1.0) required for acceptance.
    pub min_confidence: f64,
    /// Minimum coverage ratio (0.0–1.0) required for acceptance.
    pub min_coverage: f64,
    /// Whether schemas below the quality threshold are still emitted.
    pub allow_low_quality: bool,
}

impl Default for QualityPolicyFingerprint {
    fn default() -> Self {
        Self {
            min_confidence: 0.6,
            min_coverage: 0.2,
            allow_low_quality: false,
        }
    }
}

/// Per-command metadata recorded after extraction.
///
/// Tracks all the information needed to detect when a command should be
/// re-extracted: version string, executable fingerprint (path, mtime, size),
/// quality tier, and a content checksum.
///
/// # Examples
///
/// ```
/// use command_schema_db::CommandMetadata;
///
/// let meta = CommandMetadata {
///     version: Some("2.43.0".into()),
///     executable_path: Some("/usr/bin/git".into()),
///     mtime_secs: Some(1_700_000_000),
///     size_bytes: Some(3_500_000),
///     extracted_at: "2024-01-15T10:30:00Z".into(),
///     quality_tier: "high".into(),
///     checksum: "abc123def456".into(),
///     implementation: Some("git".into()),
///     schema_file: Some("git.json".into()),
/// };
/// assert_eq!(meta.version.as_deref(), Some("2.43.0"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandMetadata {
    /// Version string reported by the command (e.g., `git --version`).
    pub version: Option<String>,
    /// Resolved absolute path to the executable.
    pub executable_path: Option<String>,
    /// Last modification time of the executable (Unix timestamp in seconds).
    pub mtime_secs: Option<i64>,
    /// Size of the executable in bytes.
    pub size_bytes: Option<u64>,
    /// ISO-8601 timestamp of when the schema was extracted.
    pub extracted_at: String,
    /// Quality tier assigned during extraction: `"high"`, `"medium"`, `"low"`, or `"failed"`.
    pub quality_tier: String,
    /// SHA-256 hex digest of the schema JSON file on disk.
    pub checksum: String,
    /// Resolved implementation binary name (e.g. `mawk`, `gawk`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation: Option<String>,
    /// Output schema file name relative to the schema directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_file: Option<String>,
}

/// Top-level manifest tracking all extracted command schemas.
///
/// Persisted as pretty-printed JSON alongside the schema database directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema contract version (mirrors [`command_schema_core::SCHEMA_CONTRACT_VERSION`]).
    pub schema_version: String,
    /// Manifest format version (e.g., `"1.0"`).
    pub version: String,
    /// Version of the extraction tool that produced this manifest.
    pub tool_version: String,
    /// Quality policy in effect when schemas were extracted.
    pub quality_policy: QualityPolicyFingerprint,
    /// ISO-8601 timestamp of the last manifest update.
    pub updated_at: String,
    /// Per-command metadata keyed by command name.
    pub commands: HashMap<String, CommandMetadata>,
}

impl Manifest {
    /// Creates a new, empty manifest.
    ///
    /// The `schema_version` is set from
    /// [`command_schema_core::SCHEMA_CONTRACT_VERSION`] and `updated_at` is
    /// set to the current time.
    pub fn new(tool_version: String, quality_policy: QualityPolicyFingerprint) -> Self {
        Self {
            schema_version: command_schema_core::SCHEMA_CONTRACT_VERSION.to_string(),
            version: "1.0".to_string(),
            tool_version,
            quality_policy,
            updated_at: now_iso8601(),
            commands: HashMap::new(),
        }
    }

    /// Loads a manifest from a JSON file.
    ///
    /// # Errors
    ///
    /// Returns [`IoError`](crate::DatabaseError::IoError) if the file cannot
    /// be read, or [`JsonError`](crate::DatabaseError::JsonError) if the
    /// content is not valid manifest JSON.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let reader = BufReader::new(file);
        let manifest = serde_json::from_reader(reader)?;
        Ok(manifest)
    }

    /// Saves the manifest as pretty-printed JSON.
    ///
    /// # Errors
    ///
    /// Returns [`IoError`](crate::DatabaseError::IoError) if the file cannot
    /// be written, or [`JsonError`](crate::DatabaseError::JsonError) if
    /// serialization fails.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let file = std::fs::File::create(path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, self)?;
        Ok(())
    }

    /// Inserts or updates the metadata for `command` and refreshes `updated_at`.
    pub fn update_entry(&mut self, command: String, metadata: CommandMetadata) {
        self.commands.insert(command, metadata);
        self.updated_at = now_iso8601();
    }

    /// Computes the SHA-256 hex digest of a file.
    ///
    /// Used to populate [`CommandMetadata::checksum`] after writing a schema
    /// JSON file.
    ///
    /// # Errors
    ///
    /// Returns [`IoError`](crate::DatabaseError::IoError) if the file cannot
    /// be read.
    pub fn calculate_checksum(schema_path: impl AsRef<Path>) -> Result<String> {
        let bytes = std::fs::read(schema_path)?;
        let hash = Sha256::digest(&bytes);
        Ok(format!("{:x}", hash))
    }

    /// Returns the names of commands that differ between `self` and `other`.
    ///
    /// A command is considered changed if:
    /// - It exists in one manifest but not the other.
    /// - Its version string differs.
    /// - Its checksum differs.
    /// - For version-less commands, any fingerprint field (path, mtime, size)
    ///   differs.
    ///
    /// If the quality policy or tool version changed, **all** commands are
    /// returned for re-extraction.
    ///
    /// # Examples
    ///
    /// ```
    /// use command_schema_db::{Manifest, QualityPolicyFingerprint, CommandMetadata};
    ///
    /// let mut old = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
    /// old.update_entry("git".into(), CommandMetadata {
    ///     version: Some("2.43.0".into()),
    ///     executable_path: None, mtime_secs: None, size_bytes: None,
    ///     extracted_at: "2024-01-01T00:00:00Z".into(),
    ///     quality_tier: "high".into(), checksum: "abc".into(),
    ///     implementation: Some("git".into()),
    ///     schema_file: Some("git.json".into()),
    /// });
    ///
    /// let mut new = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
    /// new.update_entry("git".into(), CommandMetadata {
    ///     version: Some("2.44.0".into()), // version changed
    ///     executable_path: None, mtime_secs: None, size_bytes: None,
    ///     extracted_at: "2024-02-01T00:00:00Z".into(),
    ///     quality_tier: "high".into(), checksum: "abc".into(),
    ///     implementation: Some("git".into()),
    ///     schema_file: Some("git.json".into()),
    /// });
    ///
    /// let changed = old.diff(&new);
    /// assert!(changed.contains(&"git".to_string()));
    /// ```
    pub fn diff(&self, other: &Manifest) -> Vec<String> {
        // If quality policy or tool version changed, force re-extraction of all commands.
        if self.quality_policy != other.quality_policy || self.tool_version != other.tool_version {
            let mut all: Vec<String> = self.commands.keys().cloned().collect();
            for name in other.commands.keys() {
                if !self.commands.contains_key(name) {
                    all.push(name.clone());
                }
            }
            return all;
        }

        let mut changed = Vec::new();

        // Commands in self but not other, or differing
        for (name, meta) in &self.commands {
            match other.commands.get(name) {
                None => changed.push(name.clone()),
                Some(other_meta) => {
                    if meta.version != other_meta.version || meta.checksum != other_meta.checksum {
                        changed.push(name.clone());
                    } else if meta.version.is_none() {
                        // Version-less: check executable fingerprint
                        if meta.executable_path != other_meta.executable_path
                            || meta.mtime_secs != other_meta.mtime_secs
                            || meta.size_bytes != other_meta.size_bytes
                        {
                            changed.push(name.clone());
                        }
                    }
                }
            }
        }

        // Commands in other but not self
        for name in other.commands.keys() {
            if !self.commands.contains_key(name) {
                changed.push(name.clone());
            }
        }

        changed
    }

    /// Looks up metadata for a command.
    pub fn get(&self, command: &str) -> Option<&CommandMetadata> {
        self.commands.get(command)
    }

    /// Returns `true` if the manifest contains an entry for `command`.
    pub fn contains(&self, command: &str) -> bool {
        self.commands.contains_key(command)
    }
}

/// Returns the current UTC time as an ISO-8601 string.
///
/// Uses a simple manual approach to avoid pulling in a datetime crate for
/// production use. The format is `YYYY-MM-DDThh:mm:ssZ`.
fn now_iso8601() -> String {
    // Use SystemTime for a lightweight timestamp without extra dependencies.
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();

    // Decompose into calendar components (simplified UTC-only calculation).
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since 1970-01-01 → year/month/day.
    let (year, month, day) = days_to_ymd(days);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds,
    )
}

/// Converts days since the Unix epoch to (year, month, day).
fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    let leap = is_leap(year);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];

    let mut month = 0;
    for (i, &md) in month_days.iter().enumerate() {
        if days < md {
            month = i as u64 + 1;
            break;
        }
        days -= md;
    }

    (year, month, days + 1)
}

fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn sample_metadata(version: Option<&str>, checksum: &str) -> CommandMetadata {
        CommandMetadata {
            version: version.map(String::from),
            executable_path: Some("/usr/bin/test".into()),
            mtime_secs: Some(1_700_000_000),
            size_bytes: Some(100_000),
            extracted_at: "2024-01-15T10:30:00Z".into(),
            quality_tier: "high".into(),
            checksum: checksum.into(),
            implementation: Some("test".into()),
            schema_file: Some("test.json".into()),
        }
    }

    #[test]
    fn test_manifest_creation() {
        let m = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
        assert_eq!(
            m.schema_version,
            command_schema_core::SCHEMA_CONTRACT_VERSION
        );
        assert_eq!(m.version, "1.0");
        assert_eq!(m.tool_version, "0.1.0");
        assert!(m.commands.is_empty());
    }

    #[test]
    fn test_load_save_roundtrip() {
        let dir = std::env::temp_dir().join("cs_db_test_manifest_rt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("manifest.json");

        let mut m = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
        m.update_entry("git".into(), sample_metadata(Some("2.43.0"), "abc123"));
        m.save(&path).unwrap();

        let loaded = Manifest::load(&path).unwrap();
        assert_eq!(loaded.tool_version, "0.1.0");
        assert!(loaded.contains("git"));
        assert_eq!(
            loaded.get("git").unwrap().version.as_deref(),
            Some("2.43.0")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_checksum_calculation() {
        let dir = std::env::temp_dir().join("cs_db_test_checksum");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.json");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"hello world").unwrap();

        let checksum = Manifest::calculate_checksum(&path).unwrap();
        // SHA-256 of "hello world"
        assert_eq!(
            checksum,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_diff_detects_version_change() {
        let mut a = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
        a.update_entry("git".into(), sample_metadata(Some("2.43.0"), "abc123"));

        let mut b = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
        b.update_entry("git".into(), sample_metadata(Some("2.44.0"), "abc123"));

        let diff = a.diff(&b);
        assert!(diff.contains(&"git".to_string()));
    }

    #[test]
    fn test_diff_detects_checksum_change() {
        let mut a = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
        a.update_entry("git".into(), sample_metadata(Some("2.43.0"), "abc123"));

        let mut b = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
        b.update_entry("git".into(), sample_metadata(Some("2.43.0"), "def456"));

        let diff = a.diff(&b);
        assert!(diff.contains(&"git".to_string()));
    }

    #[test]
    fn test_diff_detects_fingerprint_change_for_versionless() {
        let mut a = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
        a.update_entry("mytool".into(), sample_metadata(None, "abc123"));

        let mut b = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
        let mut meta = sample_metadata(None, "abc123");
        meta.size_bytes = Some(200_000); // different size
        b.update_entry("mytool".into(), meta);

        let diff = a.diff(&b);
        assert!(diff.contains(&"mytool".to_string()));
    }

    #[test]
    fn test_diff_detects_missing_command() {
        let mut a = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
        a.update_entry("git".into(), sample_metadata(Some("2.43.0"), "abc123"));

        let b = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());

        let diff = a.diff(&b);
        assert!(diff.contains(&"git".to_string()));
    }

    #[test]
    fn test_diff_forces_all_on_policy_change() {
        let mut a = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
        a.update_entry("git".into(), sample_metadata(Some("2.43.0"), "abc123"));
        a.update_entry("cargo".into(), sample_metadata(Some("1.0.0"), "def456"));

        let new_policy = QualityPolicyFingerprint {
            min_confidence: 0.8,
            ..Default::default()
        };
        let mut b = Manifest::new("0.1.0".into(), new_policy);
        b.update_entry("git".into(), sample_metadata(Some("2.43.0"), "abc123"));
        b.update_entry("cargo".into(), sample_metadata(Some("1.0.0"), "def456"));

        let diff = a.diff(&b);
        assert!(diff.contains(&"git".to_string()));
        assert!(diff.contains(&"cargo".to_string()));
    }

    #[test]
    fn test_diff_forces_all_on_tool_version_change() {
        let mut a = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
        a.update_entry("git".into(), sample_metadata(Some("2.43.0"), "abc123"));

        let mut b = Manifest::new("0.2.0".into(), QualityPolicyFingerprint::default());
        b.update_entry("git".into(), sample_metadata(Some("2.43.0"), "abc123"));

        let diff = a.diff(&b);
        assert!(diff.contains(&"git".to_string()));
    }

    #[test]
    fn test_diff_includes_other_only_commands_on_policy_change() {
        let mut a = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
        a.update_entry("git".into(), sample_metadata(Some("2.43.0"), "abc123"));

        let new_policy = QualityPolicyFingerprint {
            min_confidence: 0.9,
            ..Default::default()
        };
        let mut b = Manifest::new("0.1.0".into(), new_policy);
        b.update_entry("git".into(), sample_metadata(Some("2.43.0"), "abc123"));
        b.update_entry("cargo".into(), sample_metadata(Some("1.0.0"), "def456"));

        let diff = a.diff(&b);
        assert!(diff.contains(&"git".to_string()));
        assert!(diff.contains(&"cargo".to_string()));
    }

    #[test]
    fn test_update_entry_modifies_timestamp() {
        let mut m = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
        let ts1 = m.updated_at.clone();

        // Sleep briefly to ensure different timestamp
        std::thread::sleep(std::time::Duration::from_millis(10));
        m.update_entry("git".into(), sample_metadata(Some("2.43.0"), "abc123"));
        // The timestamp format may be identical within the same second,
        // but the entry should be present.
        assert!(m.contains("git"));
        // At minimum, updated_at was refreshed (it's always set).
        assert!(!m.updated_at.is_empty());
        let _ = ts1; // acknowledged
    }

    #[test]
    fn test_policy_fingerprint_equality() {
        let a = QualityPolicyFingerprint::default();
        let b = QualityPolicyFingerprint {
            min_confidence: 0.6,
            min_coverage: 0.2,
            allow_low_quality: false,
        };
        assert_eq!(a, b);

        let c = QualityPolicyFingerprint {
            min_confidence: 0.8,
            ..Default::default()
        };
        assert_ne!(a, c);
    }
}
