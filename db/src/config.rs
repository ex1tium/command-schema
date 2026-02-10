//! CI configuration for schema extraction pipelines.
//!
//! Defines the YAML-serializable configuration that controls which commands are
//! extracted, quality thresholds, and extraction parallelism.
//!
//! # Example YAML
//!
//! ```yaml
//! version: "1.0"
//! allowlist:
//!   - git
//!   - docker
//!   - cargo
//! exclude:
//!   - dangerous-tool
//! quality:
//!   min_confidence: 0.6
//!   min_coverage: 0.2
//!   allow_low_quality: false
//! extraction:
//!   jobs: 4
//!   installed_only: true
//!   scan_path: false
//! ```

use std::io::{BufReader, BufWriter};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Quality thresholds for schema acceptance.
///
/// Used in [`CIConfig`] to control which extracted schemas are kept.
///
/// # Examples
///
/// ```
/// # use command_schema_db::QualityConfig;
/// let q = QualityConfig {
///     min_confidence: 0.7,
///     min_coverage: 0.3,
///     allow_low_quality: false,
/// };
/// assert_eq!(q.min_confidence, 0.7);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityConfig {
    /// Minimum confidence score (0.0–1.0) for a schema to be accepted.
    pub min_confidence: f64,
    /// Minimum coverage ratio (0.0–1.0) for a schema to be accepted.
    pub min_coverage: f64,
    /// Whether to emit schemas that fall below the quality threshold.
    pub allow_low_quality: bool,
}

/// Settings controlling how extraction is performed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionConfig {
    /// Number of parallel extraction jobs.
    pub jobs: usize,
    /// Only extract commands that are currently installed.
    pub installed_only: bool,
    /// Scan `$PATH` directories for additional commands.
    pub scan_path: bool,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            jobs: 4,
            installed_only: true,
            scan_path: false,
        }
    }
}

/// Top-level CI pipeline configuration.
///
/// Loaded from a YAML file (typically `.command-schema.yml` in the repository
/// root) to control batch extraction runs.
///
/// # Examples
///
/// ```no_run
/// use command_schema_db::CIConfig;
///
/// let config = CIConfig::load(".command-schema.yml").unwrap();
/// if config.is_allowed("git") {
///     println!("git extraction is allowed");
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CIConfig {
    /// Configuration format version (e.g., `"1.0"`).
    pub version: String,
    /// Commands to extract (empty = extract all discovered commands).
    pub allowlist: Vec<String>,
    /// Commands to explicitly skip.
    pub exclude: Vec<String>,
    /// Quality acceptance thresholds.
    pub quality: QualityConfig,
    /// Extraction process settings.
    pub extraction: ExtractionConfig,
}

impl CIConfig {
    /// Loads configuration from a YAML file.
    ///
    /// # Errors
    ///
    /// Returns [`IoError`](crate::DatabaseError::IoError) if the file cannot
    /// be read, or [`YamlError`](crate::DatabaseError::YamlError) if parsing
    /// fails.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let reader = BufReader::new(file);
        let config = serde_yaml::from_reader(reader)?;
        Ok(config)
    }

    /// Saves the configuration as YAML.
    ///
    /// # Errors
    ///
    /// Returns [`IoError`](crate::DatabaseError::IoError) if the file cannot
    /// be written, or [`YamlError`](crate::DatabaseError::YamlError) if
    /// serialization fails.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let file = std::fs::File::create(path)?;
        let writer = BufWriter::new(file);
        serde_yaml::to_writer(writer, self)?;
        Ok(())
    }

    /// Returns `true` if `command` is in the exclusion list.
    pub fn is_excluded(&self, command: &str) -> bool {
        self.exclude.iter().any(|c| c == command)
    }

    /// Returns `true` if `command` is allowed.
    ///
    /// When the allowlist is empty, all non-excluded commands are implicitly
    /// allowed. When the allowlist is non-empty, the command must be present
    /// in it. Exclusions are always honored.
    ///
    /// # Examples
    ///
    /// ```
    /// # let yaml = r#"
    /// # version: "1.0"
    /// # allowlist: [git, docker]
    /// # exclude: [dangerous]
    /// # quality: { min_confidence: 0.6, min_coverage: 0.2, allow_low_quality: false }
    /// # extraction: { jobs: 4, installed_only: true, scan_path: false }
    /// # "#;
    /// # let config: command_schema_db::CIConfig = serde_yaml::from_str(yaml).unwrap();
    /// assert!(config.is_allowed("git"));
    /// assert!(!config.is_allowed("unknown"));
    /// assert!(!config.is_allowed("dangerous"));
    /// ```
    pub fn is_allowed(&self, command: &str) -> bool {
        if self.is_excluded(command) {
            return false;
        }
        if self.allowlist.is_empty() {
            return true;
        }
        self.allowlist.iter().any(|c| c == command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_yaml() -> &'static str {
        r#"
version: "1.0"
allowlist:
  - git
  - docker
  - cargo
exclude:
  - dangerous-tool
quality:
  min_confidence: 0.7
  min_coverage: 0.3
  allow_low_quality: true
extraction:
  jobs: 8
  installed_only: false
  scan_path: true
"#
    }

    fn minimal_yaml() -> &'static str {
        r#"
version: "1.0"
allowlist: []
exclude: []
quality:
  min_confidence: 0.6
  min_coverage: 0.2
  allow_low_quality: false
extraction:
  jobs: 4
  installed_only: true
  scan_path: false
"#
    }

    #[test]
    fn test_deserialize_complete() {
        let config: CIConfig = serde_yaml::from_str(sample_yaml()).unwrap();
        assert_eq!(config.version, "1.0");
        assert_eq!(config.allowlist, vec!["git", "docker", "cargo"]);
        assert_eq!(config.exclude, vec!["dangerous-tool"]);
        assert_eq!(config.quality.min_confidence, 0.7);
        assert_eq!(config.quality.min_coverage, 0.3);
        assert!(config.quality.allow_low_quality);
        assert_eq!(config.extraction.jobs, 8);
        assert!(!config.extraction.installed_only);
        assert!(config.extraction.scan_path);
    }

    #[test]
    fn test_deserialize_minimal() {
        let config: CIConfig = serde_yaml::from_str(minimal_yaml()).unwrap();
        assert_eq!(config.version, "1.0");
        assert!(config.allowlist.is_empty());
        assert!(config.exclude.is_empty());
    }

    #[test]
    fn test_is_excluded() {
        let config: CIConfig = serde_yaml::from_str(sample_yaml()).unwrap();
        assert!(config.is_excluded("dangerous-tool"));
        assert!(!config.is_excluded("git"));
    }

    #[test]
    fn test_is_allowed() {
        let config: CIConfig = serde_yaml::from_str(sample_yaml()).unwrap();
        assert!(config.is_allowed("git"));
        assert!(config.is_allowed("docker"));
        assert!(!config.is_allowed("unknown"));
    }

    #[test]
    fn test_empty_allowlist_allows_all_non_excluded() {
        let config: CIConfig = serde_yaml::from_str(minimal_yaml()).unwrap();
        assert!(config.allowlist.is_empty());
        // Any command should be allowed when allowlist is empty
        assert!(config.is_allowed("git"));
        assert!(config.is_allowed("docker"));
        assert!(config.is_allowed("random-tool"));
    }

    #[test]
    fn test_empty_allowlist_still_honors_exclusions() {
        let yaml = r#"
version: "1.0"
allowlist: []
exclude:
  - dangerous-tool
quality:
  min_confidence: 0.6
  min_coverage: 0.2
  allow_low_quality: false
extraction:
  jobs: 4
  installed_only: true
  scan_path: false
"#;
        let config: CIConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.is_allowed("git"));
        assert!(!config.is_allowed("dangerous-tool"));
    }

    #[test]
    fn test_nonempty_allowlist_excludes_unlisted() {
        let config: CIConfig = serde_yaml::from_str(sample_yaml()).unwrap();
        assert!(config.is_allowed("git"));
        assert!(!config.is_allowed("unknown"));
        // Excluded command is rejected even if hypothetically in allowlist
        assert!(!config.is_allowed("dangerous-tool"));
    }

    #[test]
    fn test_load_save_roundtrip() {
        let dir = std::env::temp_dir().join("cs_db_test_config_rt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yml");

        let original: CIConfig = serde_yaml::from_str(sample_yaml()).unwrap();
        original.save(&path).unwrap();

        let loaded = CIConfig::load(&path).unwrap();
        assert_eq!(loaded.version, original.version);
        assert_eq!(loaded.allowlist, original.allowlist);
        assert_eq!(loaded.exclude, original.exclude);
        assert_eq!(
            loaded.quality.min_confidence,
            original.quality.min_confidence
        );
        assert_eq!(loaded.extraction.jobs, original.extraction.jobs);

        std::fs::remove_dir_all(&dir).ok();
    }
}
