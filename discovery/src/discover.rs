//! Discovery helpers and file workflows for schema extraction.

use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use command_schema_core::{CommandSchema, SchemaPackage, validate_package, validate_schema};

use crate::extractor::{
    ExtractionQualityPolicy, command_exists, extract_command_schema_with_report_and_policy,
};
use crate::report::{ExtractionReport, ExtractionReportBundle};

/// Typed error for schema discovery file operations.
#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    /// Filesystem I/O failure.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON parsing or serialization failure.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Schema or package validation failure.
    #[error("validation failed: {0}")]
    Validation(String),

    /// Invalid or missing input (e.g. non-existent path, wrong extension).
    #[error("{0}")]
    InvalidInput(String),
}

/// Default allowlist for schema extraction.
pub const DEFAULT_ALLOWLIST: &[&str] = &[
    "awk",
    "bash",
    "cat",
    "cd",
    "chmod",
    "chown",
    "cp",
    "curl",
    "docker",
    "du",
    "echo",
    "env",
    "find",
    "git",
    "grep",
    "head",
    "jq",
    "kill",
    "kubectl",
    "less",
    "ln",
    "ls",
    "make",
    "mkdir",
    "mv",
    "nano",
    "npm",
    "pnpm",
    "ps",
    "pwd",
    "rg",
    "rm",
    "rmdir",
    "sed",
    "ssh",
    "sudo",
    "systemctl",
    "tail",
    "tar",
    "touch",
    "vim",
    "wget",
    "whoami",
    "xargs",
    "yarn",
    "cargo",
    "rustc",
    "go",
    "python",
    "python3",
];

/// Tool discovery and extraction configuration.
#[derive(Debug, Clone, Default)]
pub struct DiscoverConfig {
    /// Explicit command names supplied by the caller.
    pub commands: Vec<String>,
    /// Include commands from [`DEFAULT_ALLOWLIST`].
    pub use_allowlist: bool,
    /// Include executables found on `PATH`.
    pub scan_path: bool,
    /// Commands to suppress from all discovery sources.
    pub excluded_commands: Vec<String>,
    /// Quality policy used to decide whether extracted schemas are accepted.
    pub quality_policy: ExtractionQualityPolicy,
    /// Only include commands that are installed on the system.
    pub installed_only: bool,
    /// Number of parallel extraction jobs (`None` = adaptive default).
    pub jobs: Option<usize>,
    /// Directory for caching extraction results. `None` disables caching.
    pub cache_dir: Option<PathBuf>,
}

/// Aggregated output from a discovery + extraction run.
#[derive(Debug, Clone)]
pub struct DiscoverOutcome {
    /// Package containing all successfully extracted command schemas.
    pub package: SchemaPackage,
    /// Command names that failed extraction.
    pub failures: Vec<String>,
    /// Non-fatal extraction warnings.
    pub warnings: Vec<String>,
    /// Per-command extraction reports.
    pub reports: Vec<ExtractionReport>,
}

/// Returns a deterministic, deduplicated command list based on config.
pub fn discover_tools(config: &DiscoverConfig) -> Vec<String> {
    let excluded: BTreeSet<&str> = config
        .excluded_commands
        .iter()
        .map(String::as_str)
        .filter(|cmd| !cmd.is_empty())
        .collect();

    let mut commands: BTreeSet<String> = BTreeSet::new();

    for cmd in &config.commands {
        let trimmed = cmd.trim();
        if trimmed.is_empty() || excluded.contains(trimmed) {
            continue;
        }
        commands.insert(trimmed.to_string());
    }

    if config.use_allowlist {
        for &cmd in DEFAULT_ALLOWLIST {
            if excluded.contains(cmd) {
                continue;
            }
            if command_exists(cmd) {
                commands.insert(cmd.to_string());
            }
        }
    }

    if config.scan_path {
        for cmd in path_executables() {
            if !is_scan_path_probe_candidate(cmd.as_str()) {
                continue;
            }
            if !excluded.contains(cmd.as_str()) {
                commands.insert(cmd);
            }
        }
    }

    if config.installed_only {
        commands.retain(|cmd| command_exists(cmd));
    }

    commands.into_iter().collect()
}

/// Discovers commands and extracts schemas into a package.
pub fn discover_and_extract(config: &DiscoverConfig, version: &str) -> DiscoverOutcome {
    let commands = discover_tools(config);
    let policy = config.quality_policy;
    let cache = config
        .cache_dir
        .as_ref()
        .map(|dir| crate::cache::SchemaCache::new(dir.clone()));

    let extract_one = |command: &str| -> (String, crate::extractor::ExtractionRun) {
        // Build cache key once (includes policy thresholds) and reuse for
        // both lookup and store.
        let cache_key = cache
            .as_ref()
            .and_then(|_| crate::cache::build_cache_key(command, &policy));

        // Try cache lookup.  When a cached entry exists, compare its
        // stored version against a quick `--version` probe so that
        // binary upgrades that don't change mtime/size still invalidate.
        if let Some(ref cache) = cache {
            if let Some(ref key) = cache_key {
                if let Some(entry) = cache.get(key) {
                    let current_version = crate::cache::detect_quick_version(command);
                    let version_matches = match (&entry.detected_version, &current_version) {
                        (Some(cached), Some(current)) => cached == current,
                        (None, None) => true,
                        _ => false,
                    };
                    if version_matches {
                        let run = crate::extractor::ExtractionRun {
                            result: command_schema_core::ExtractionResult {
                                schema: entry.schema,
                                raw_output: String::new(),
                                detected_format: None,
                                warnings: Vec::new(),
                                success: entry.report.success,
                            },
                            report: entry.report,
                        };
                        return (command.to_string(), run);
                    }
                    // Version mismatch → treat as cache miss, fall through
                    // to re-extract.
                }
            }
        }

        let run = extract_command_schema_with_report_and_policy(command, policy);

        // Store in cache using the same key built above, including the
        // detected version and probe mode for future invalidation checks.
        if let Some(ref cache) = cache {
            if let Some(key) = cache_key {
                let detected_version = run.result.schema.as_ref().and_then(|s| s.version.clone());
                let probe_mode = run.report.selected_format.clone();
                cache.put(
                    key,
                    run.result.schema.clone(),
                    run.report.clone(),
                    detected_version,
                    probe_mode,
                );
            }
        }

        (command.to_string(), run)
    };

    // Collect extraction results in parallel with an adaptive default that
    // avoids oversubscribing process-spawn heavy workloads.
    let results: Vec<(String, crate::extractor::ExtractionRun)> = {
        use rayon::prelude::*;
        let jobs = config
            .jobs
            .filter(|jobs| *jobs > 0)
            .unwrap_or_else(|| default_parallel_jobs(commands.len()));
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build()
            .expect("failed to build rayon thread pool");

        pool.install(|| {
            commands
                .par_iter()
                .map(|command| extract_one(command))
                .collect()
        })
    };

    // Sort by command name for deterministic output.
    let mut sorted_results = results;
    sorted_results.sort_by(|(a, _), (b, _)| a.cmp(b));

    let mut package = SchemaPackage::new(version, Utc::now().to_rfc3339());
    let mut failures = Vec::new();
    let mut warnings = Vec::new();
    let mut reports = Vec::new();

    for (command, run) in sorted_results {
        let result = run.result;
        let command_label = command.clone();

        if run.report.accepted_for_suggestions {
            if let Some(mut schema) = result.schema {
                schema.schema_version =
                    Some(command_schema_core::SCHEMA_CONTRACT_VERSION.to_string());
                package.schemas.push(schema);
            } else {
                failures.push(command);
            }
        } else {
            failures.push(command);
        }

        warnings.extend(
            result
                .warnings
                .into_iter()
                .map(|warning| format!("{}: {}", command_label, warning)),
        );
        reports.push(run.report);
    }

    DiscoverOutcome {
        package,
        failures,
        warnings,
        reports,
    }
}

fn default_parallel_jobs(command_count: usize) -> usize {
    let cpu_count = std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(4);
    let adaptive_cap = if command_count >= 500 { 8 } else { 12 };
    cpu_count.min(adaptive_cap).max(1).min(command_count.max(1))
}

/// Summarizes failure code distribution from extraction reports.
pub fn failure_code_summary(
    reports: &[crate::report::ExtractionReport],
) -> Vec<(crate::report::FailureCode, usize)> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<String, (crate::report::FailureCode, usize)> = BTreeMap::new();
    for report in reports {
        if let Some(code) = report.failure_code {
            let key = code.to_string();
            counts
                .entry(key)
                .and_modify(|(_, count)| *count += 1)
                .or_insert((code, 1));
        }
    }
    counts.into_values().collect()
}

/// Builds a serializable bundle report for a discovery run.
pub fn build_report_bundle(
    version: &str,
    reports: Vec<ExtractionReport>,
    failures: Vec<String>,
) -> ExtractionReportBundle {
    ExtractionReportBundle {
        schema_version: Some(command_schema_core::SCHEMA_CONTRACT_VERSION.to_string()),
        generated_at: Utc::now().to_rfc3339(),
        version: version.to_string(),
        reports,
        failures,
    }
}

/// Collects JSON schema file paths from input files and/or directories.
pub fn collect_schema_paths(inputs: &[PathBuf]) -> Result<Vec<PathBuf>, DiscoverError> {
    if inputs.is_empty() {
        return Err(DiscoverError::InvalidInput(
            "No schema paths were provided".to_string(),
        ));
    }

    let mut paths = BTreeSet::new();

    for input in inputs {
        if input.is_dir() {
            let entries = fs::read_dir(input)?;

            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                let is_json = path.extension() == Some(OsStr::new("json"));
                let is_report = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == "extraction-report.json");
                if is_json && !is_report {
                    paths.insert(path);
                }
            }
            continue;
        }

        if input.is_file() {
            if input.extension() != Some(OsStr::new("json")) {
                return Err(DiscoverError::InvalidInput(format!(
                    "Schema file '{}' must end in .json",
                    input.display()
                )));
            }
            paths.insert(input.clone());
            continue;
        }

        return Err(DiscoverError::InvalidInput(format!(
            "Schema path '{}' does not exist",
            input.display(),
        )));
    }

    if paths.is_empty() {
        return Err(DiscoverError::InvalidInput(
            "No schema JSON files found in provided paths".to_string(),
        ));
    }

    Ok(paths.into_iter().collect())
}

/// Loads and validates all command schemas from files.
pub fn load_and_validate_schemas(paths: &[PathBuf]) -> Result<Vec<CommandSchema>, DiscoverError> {
    let mut schemas = Vec::with_capacity(paths.len());

    for path in paths {
        let raw = fs::read_to_string(path)?;
        let schema: CommandSchema = serde_json::from_str(&raw)?;

        let errors = validate_schema(&schema);
        if let Some(first) = errors.first() {
            return Err(DiscoverError::Validation(format!(
                "Schema validation failed for '{}': {first}",
                path.display()
            )));
        }

        schemas.push(schema);
    }

    Ok(schemas)
}

/// Bundles multiple schema files into a validated [`SchemaPackage`].
pub fn bundle_schema_files(
    paths: &[PathBuf],
    version: &str,
    name: Option<String>,
    description: Option<String>,
) -> Result<SchemaPackage, DiscoverError> {
    let schemas = load_and_validate_schemas(paths)?;

    let mut package = SchemaPackage::new(version, Utc::now().to_rfc3339());
    package.name = name;
    package.description = description;
    package.schemas = schemas;

    let errors = validate_package(&package);
    if let Some(first) = errors.first() {
        return Err(DiscoverError::Validation(format!(
            "Schema package validation failed: {first}"
        )));
    }

    Ok(package)
}

fn path_executables() -> Vec<String> {
    let Some(path_env) = env::var_os("PATH") else {
        return Vec::new();
    };

    let mut commands = BTreeSet::new();

    for dir in env::split_paths(&path_env) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !is_executable(&path) {
                continue;
            }

            if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
                commands.insert(name.to_string());
            }
        }
    }

    commands.into_iter().collect()
}

fn is_scan_path_probe_candidate(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();

    if lower.starts_with("xdg-") {
        return false;
    }

    !matches!(
        lower.as_str(),
        "open"
            | "browse"
            | "sensible-browser"
            | "xmessage"
            | "xhost"
            | "xsetmode"
            | "xsetpointer"
            | "xkeystone"
    )
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };

    metadata.is_file() && (metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use command_schema_core::{FlagSchema, SchemaSource};

    use super::*;

    #[test]
    fn test_discover_tools_dedupes_and_applies_exclusions() {
        let config = DiscoverConfig {
            commands: vec!["git".to_string(), "git".to_string(), "cargo".to_string()],
            use_allowlist: false,
            scan_path: false,
            excluded_commands: vec!["cargo".to_string()],
            quality_policy: ExtractionQualityPolicy::default(),
            installed_only: false,
            jobs: None,
            cache_dir: None,
        };

        assert_eq!(discover_tools(&config), vec!["git".to_string()]);
    }

    #[test]
    fn test_collect_schema_paths_from_dir_filters_non_json() {
        let root = unique_tmp_dir();
        fs::create_dir_all(&root).unwrap();

        let json_path = root.join("git.json");
        let report_path = root.join("extraction-report.json");
        let txt_path = root.join("notes.txt");
        fs::write(&json_path, "{}").unwrap();
        fs::write(&report_path, "{}").unwrap();
        fs::write(&txt_path, "ignore").unwrap();

        let paths = collect_schema_paths(&[root]).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], json_path);
    }

    #[test]
    fn test_bundle_schema_files_rejects_duplicate_commands() {
        let root = unique_tmp_dir();
        fs::create_dir_all(&root).unwrap();

        let schema = CommandSchema {
            schema_version: None,
            command: "git".to_string(),
            description: Some("Git tool".to_string()),
            global_flags: vec![FlagSchema::boolean(Some("-v"), Some("--verbose"))],
            subcommands: Vec::new(),
            positional: Vec::new(),
            source: SchemaSource::Bootstrap,
            confidence: 1.0,
            version: None,
        };

        let file_a = root.join("a.json");
        let file_b = root.join("b.json");
        let raw = serde_json::to_string_pretty(&schema).unwrap();
        fs::write(&file_a, &raw).unwrap();
        fs::write(&file_b, &raw).unwrap();

        let result = bundle_schema_files(&[file_a, file_b], "1.0.0", None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_report_bundle_populates_metadata() {
        let bundle = build_report_bundle("1.2.3", Vec::new(), vec!["npm".to_string()]);
        assert_eq!(bundle.version, "1.2.3");
        assert_eq!(bundle.failures, vec!["npm".to_string()]);
        assert!(bundle.generated_at.contains('T'));
        assert_eq!(
            bundle.schema_version,
            Some(command_schema_core::SCHEMA_CONTRACT_VERSION.to_string())
        );
    }

    #[test]
    fn test_scan_path_probe_candidate_filters_gui_launchers() {
        assert!(!is_scan_path_probe_candidate("xdg-open"));
        assert!(!is_scan_path_probe_candidate("xmessage"));
        assert!(!is_scan_path_probe_candidate("open"));
        assert!(!is_scan_path_probe_candidate("sensible-browser"));
        assert!(is_scan_path_probe_candidate("awk"));
        assert!(is_scan_path_probe_candidate("cargo"));
    }

    #[test]
    fn test_default_parallel_jobs_is_non_zero_and_bounded_by_workload() {
        assert_eq!(default_parallel_jobs(0), 1);
        assert!(default_parallel_jobs(1) >= 1);
        assert!(default_parallel_jobs(1) <= 1);
        assert!(default_parallel_jobs(2000) <= 12);
    }

    #[test]
    fn test_default_parallel_jobs_uses_tighter_cap_for_large_workloads() {
        assert!(default_parallel_jobs(500) <= 8);
        assert!(default_parallel_jobs(2000) <= 8);
    }

    fn unique_tmp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("wrashpty-schema-discovery-{nanos}"))
    }
}
