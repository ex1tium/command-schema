use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use command_schema_discovery::discover::{
    DiscoverConfig, build_report_bundle, bundle_schema_files, collect_schema_paths,
    discover_and_extract, failure_code_summary, load_and_validate_schemas,
};
use command_schema_discovery::extractor::{
    DEFAULT_MIN_CONFIDENCE, DEFAULT_MIN_COVERAGE, ExtractionQualityPolicy,
};

const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// CLI-specific output format enum with clap argument parsing support.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum CliOutputFormat {
    Json,
    Yaml,
    Markdown,
    Table,
}

impl From<CliOutputFormat> for command_schema_discovery::output::OutputFormat {
    fn from(fmt: CliOutputFormat) -> Self {
        match fmt {
            CliOutputFormat::Json => Self::Json,
            CliOutputFormat::Yaml => Self::Yaml,
            CliOutputFormat::Markdown => Self::Markdown,
            CliOutputFormat::Table => Self::Table,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "schema-discover")]
#[command(about = "Offline command schema discovery and bundling")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Extract command schemas from local tool help output.
    Extract(ExtractArgs),
    /// Validate one or more schema JSON files.
    Validate(ValidateArgs),
    /// Bundle schema JSON files into a SchemaPackage file.
    Bundle(BundleArgs),
    /// Parse help text from stdin without executing commands.
    ParseStdin(ParseStdinArgs),
    /// Parse help text from a file without executing commands.
    ParseFile(ParseFileArgs),
    /// CI-optimized extraction with manifest-based version tracking and parallel extraction.
    CiExtract(CiExtractArgs),
    /// SQLite database migration and seeding operations.
    Migrate(MigrateArgs),
}

#[derive(Debug, Args)]
struct ExtractArgs {
    /// Comma-separated explicit commands (e.g. git,docker,cargo).
    #[arg(long)]
    commands: Option<String>,
    /// Include installed commands from the curated allowlist.
    #[arg(long)]
    allowlist: bool,
    /// Include executables discovered on PATH.
    #[arg(long)]
    scan_path: bool,
    /// Comma-separated commands to exclude.
    #[arg(long)]
    exclude: Option<String>,
    /// Output directory for per-command JSON files.
    #[arg(long)]
    output: PathBuf,
    /// Minimum schema confidence (0.0-1.0) required for acceptance.
    #[arg(long, default_value_t = DEFAULT_MIN_CONFIDENCE)]
    min_confidence: f64,
    /// Minimum parser coverage (0.0-1.0) required for acceptance.
    #[arg(long, default_value_t = DEFAULT_MIN_COVERAGE)]
    min_coverage: f64,
    /// Keep low-quality schemas instead of rejecting them.
    #[arg(long)]
    allow_low_quality: bool,
    /// Only extract schemas for commands installed on the system.
    #[arg(long)]
    installed_only: bool,
    /// Number of parallel extraction jobs (default: number of CPUs).
    #[arg(long)]
    jobs: Option<usize>,
    /// Directory for caching extraction results.
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    /// Disable caching entirely.
    #[arg(long)]
    no_cache: bool,
    /// Output format for schema and report files (default: json).
    #[arg(long, default_value = "json")]
    format: CliOutputFormat,
}

#[derive(Debug, Args)]
struct ValidateArgs {
    /// Schema files and/or directories containing schema JSON files.
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct BundleArgs {
    /// Schema files and/or directories containing schema JSON files.
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
    /// Output JSON bundle path.
    #[arg(long)]
    output: PathBuf,
    /// Optional bundle name metadata.
    #[arg(long)]
    name: Option<String>,
    /// Optional bundle description metadata.
    #[arg(long)]
    description: Option<String>,
}

#[derive(Debug, Args)]
struct ParseStdinArgs {
    /// Command name for the help text being parsed.
    #[arg(long)]
    command: String,
    /// Output both schema and extraction report.
    #[arg(long)]
    with_report: bool,
    /// Output format.
    #[arg(long, default_value = "json")]
    format: CliOutputFormat,
}

#[derive(Debug, Args)]
struct ParseFileArgs {
    /// Command name for the help text being parsed.
    #[arg(long)]
    command: String,
    /// Path to file containing help text.
    #[arg(long)]
    input: PathBuf,
    /// Output both schema and extraction report.
    #[arg(long)]
    with_report: bool,
    /// Output format.
    #[arg(long, default_value = "json")]
    format: CliOutputFormat,
}

#[derive(Debug, Args)]
struct CiExtractArgs {
    /// Path to ci-config.yaml.
    #[arg(long)]
    config: PathBuf,
    /// Path to manifest.json.
    #[arg(long)]
    manifest: PathBuf,
    /// Output directory for schemas.
    #[arg(long)]
    output: PathBuf,
    /// Force re-extraction ignoring version checks.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct MigrateArgs {
    #[command(subcommand)]
    operation: MigrateOperation,
}

#[derive(Debug, Subcommand)]
enum MigrateOperation {
    /// Create schema tables in the database.
    Up(MigrateUpArgs),
    /// Drop schema tables from the database.
    Down(MigrateDownArgs),
    /// Seed the database with JSON schemas from a directory.
    Seed(MigrateSeedArgs),
    /// Drop tables, recreate, and reseed from a directory.
    Refresh(MigrateRefreshArgs),
    /// Show migration and table status.
    Status(MigrateStatusArgs),
}

#[derive(Debug, Args)]
struct MigrateUpArgs {
    /// Database file path.
    #[arg(long)]
    db: PathBuf,
    /// Table prefix.
    #[arg(long)]
    prefix: String,
}

#[derive(Debug, Args)]
struct MigrateDownArgs {
    /// Database file path.
    #[arg(long)]
    db: PathBuf,
    /// Table prefix.
    #[arg(long)]
    prefix: String,
}

#[derive(Debug, Args)]
struct MigrateSeedArgs {
    /// Database file path.
    #[arg(long)]
    db: PathBuf,
    /// Table prefix.
    #[arg(long)]
    prefix: String,
    /// Source directory with JSON schemas.
    #[arg(long)]
    source: PathBuf,
}

#[derive(Debug, Args)]
struct MigrateRefreshArgs {
    /// Database file path.
    #[arg(long)]
    db: PathBuf,
    /// Table prefix.
    #[arg(long)]
    prefix: String,
    /// Source directory with JSON schemas.
    #[arg(long)]
    source: PathBuf,
}

#[derive(Debug, Args)]
struct MigrateStatusArgs {
    /// Database file path.
    #[arg(long)]
    db: PathBuf,
    /// Table prefix.
    #[arg(long)]
    prefix: String,
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Extract(args) => run_extract(args),
        Command::Validate(args) => run_validate(args),
        Command::Bundle(args) => run_bundle(args),
        Command::ParseStdin(args) => run_parse_stdin(args),
        Command::ParseFile(args) => run_parse_file(args),
        Command::CiExtract(args) => run_ci_extract(args),
        Command::Migrate(args) => run_migrate(args),
    };

    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run_extract(args: ExtractArgs) -> Result<(), String> {
    let commands = parse_csv_list(args.commands);
    let excluded_commands = parse_csv_list(args.exclude);

    if commands.is_empty() && !args.allowlist && !args.scan_path {
        return Err(
            "Specify at least one discovery source: --commands, --allowlist, or --scan-path"
                .to_string(),
        );
    }
    if !(0.0..=1.0).contains(&args.min_confidence) {
        return Err("--min-confidence must be between 0.0 and 1.0".to_string());
    }
    if !(0.0..=1.0).contains(&args.min_coverage) {
        return Err("--min-coverage must be between 0.0 and 1.0".to_string());
    }

    fs::create_dir_all(&args.output).map_err(|err| {
        format!(
            "Failed to create output directory '{}': {err}",
            args.output.display()
        )
    })?;

    let config = DiscoverConfig {
        commands,
        use_allowlist: args.allowlist,
        scan_path: args.scan_path,
        excluded_commands,
        quality_policy: ExtractionQualityPolicy {
            min_confidence: args.min_confidence,
            min_coverage: args.min_coverage,
            allow_low_quality: args.allow_low_quality,
        },
        installed_only: args.installed_only,
        jobs: args.jobs,
        cache_dir: if args.no_cache {
            None
        } else {
            Some(
                args.cache_dir
                    .unwrap_or_else(command_schema_discovery::cache::SchemaCache::default_dir),
            )
        },
    };

    let format: command_schema_discovery::output::OutputFormat = args.format.into();
    let outcome = discover_and_extract(&config, PACKAGE_VERSION);

    let ext = format_extension(format);
    let reports_by_command: HashMap<&str, &command_schema_discovery::report::ExtractionReport> =
        outcome
            .reports
            .iter()
            .map(|report| (report.command.as_str(), report))
            .collect();

    let mut written = 0usize;
    for schema in &outcome.package.schemas {
        let report = reports_by_command.get(schema.command.as_str()).copied();
        let stem = schema_output_stem(schema, report);
        let path = args.output.join(format!("{stem}.{ext}"));
        let raw = command_schema_discovery::output::format_schema(schema, format)?;
        fs::write(&path, raw)
            .map_err(|err| format!("Failed to write '{}': {err}", path.display()))?;
        written += 1;
    }

    println!("Extracted and wrote {written} schema file(s).");

    let report_bundle =
        build_report_bundle(PACKAGE_VERSION, outcome.reports, outcome.failures.clone());
    let report_path = args.output.join(format!("extraction-report.{ext}"));
    let report_raw = format_report_bundle(&report_bundle, format)?;
    fs::write(&report_path, report_raw)
        .map_err(|err| format!("Failed to write '{}': {err}", report_path.display()))?;

    if !outcome.failures.is_empty() {
        let summary = failure_code_summary(&report_bundle.reports);
        if summary.is_empty() {
            eprintln!(
                "{} extraction failure(s): {}",
                outcome.failures.len(),
                outcome.failures.join(", ")
            );
        } else {
            let breakdown: Vec<String> = summary
                .iter()
                .map(|(code, count)| format!("{count} {code}"))
                .collect();
            eprintln!(
                "{} extraction failure(s) ({}): {}",
                outcome.failures.len(),
                breakdown.join(", "),
                outcome.failures.join(", ")
            );
        }
    }

    if !outcome.warnings.is_empty() {
        eprintln!(
            "{} warning(s) emitted during extraction.",
            outcome.warnings.len()
        );
    }

    Ok(())
}

fn run_validate(args: ValidateArgs) -> Result<(), String> {
    let paths = collect_schema_paths(&args.inputs).map_err(|e| e.to_string())?;
    let schemas = load_and_validate_schemas(&paths).map_err(|e| e.to_string())?;
    println!(
        "Validated {} schema file(s) for {} command(s).",
        paths.len(),
        schemas.len()
    );
    Ok(())
}

fn run_bundle(args: BundleArgs) -> Result<(), String> {
    let paths = collect_schema_paths(&args.inputs).map_err(|e| e.to_string())?;
    let package = bundle_schema_files(&paths, PACKAGE_VERSION, args.name, args.description)
        .map_err(|e| e.to_string())?;

    if let Some(parent) = args.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "Failed to create output directory '{}': {err}",
                    parent.display()
                )
            })?;
        }
    }

    let raw = serde_json::to_string_pretty(&package)
        .map_err(|err| format!("Failed to serialize schema bundle: {err}"))?;
    fs::write(&args.output, raw)
        .map_err(|err| format!("Failed to write '{}': {err}", args.output.display()))?;

    println!(
        "Bundled {} schema(s) into '{}'.",
        package.schema_count(),
        args.output.display()
    );

    Ok(())
}

fn run_parse_stdin(args: ParseStdinArgs) -> Result<(), String> {
    let mut help_text = String::new();
    std::io::stdin()
        .read_to_string(&mut help_text)
        .map_err(|err| format!("Failed to read stdin: {err}"))?;
    run_parse_help_text(
        &args.command,
        &help_text,
        args.with_report,
        args.format.into(),
    )
}

fn run_parse_file(args: ParseFileArgs) -> Result<(), String> {
    let help_text = fs::read_to_string(&args.input)
        .map_err(|err| format!("Failed to read '{}': {err}", args.input.display()))?;
    run_parse_help_text(
        &args.command,
        &help_text,
        args.with_report,
        args.format.into(),
    )
}

fn run_parse_help_text(
    command: &str,
    help_text: &str,
    with_report: bool,
    format: command_schema_discovery::output::OutputFormat,
) -> Result<(), String> {
    use command_schema_discovery::output::{OutputFormat, format_report, format_schema};

    if with_report {
        let run = command_schema_discovery::parse_help_text_with_report(
            command,
            help_text,
            ExtractionQualityPolicy::permissive(),
        );

        #[derive(serde::Serialize)]
        struct ParseOutput {
            #[serde(skip_serializing_if = "Option::is_none")]
            schema: Option<command_schema_core::CommandSchema>,
            report: command_schema_discovery::report::ExtractionReport,
        }

        let output = ParseOutput {
            schema: run.result.schema.clone(),
            report: run.report.clone(),
        };

        match format {
            OutputFormat::Json => {
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| format!("Failed to serialize output: {e}"))?;
                println!("{json}");
            }
            OutputFormat::Yaml => {
                let yaml = serde_yaml::to_string(&output)
                    .map_err(|e| format!("Failed to serialize output: {e}"))?;
                println!("{yaml}");
            }
            _ => {
                if let Some(ref schema) = run.result.schema {
                    print!("{}", format_schema(schema, format)?);
                }
                print!("{}", format_report(&run.report, format)?);
            }
        }
    } else {
        let result = command_schema_discovery::parse_help_text(command, help_text);
        match result.schema {
            Some(schema) => {
                let output = format_schema(&schema, format)?;
                println!("{output}");
            }
            None => {
                return Err(format!(
                    "Failed to parse help text for '{}': {}",
                    command,
                    result.warnings.join("; ")
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ci-extract command
// ---------------------------------------------------------------------------

fn run_ci_extract(args: CiExtractArgs) -> Result<(), String> {
    use rayon::prelude::*;
    use std::time::UNIX_EPOCH;

    // 1. Load CI config from YAML
    let config = command_schema_db::CIConfig::load(&args.config)
        .map_err(|e| format!("Failed to load CI config '{}': {e}", args.config.display()))?;

    // 2. Load or create manifest
    let mut manifest = if args.manifest.exists() {
        command_schema_db::Manifest::load(&args.manifest)
            .map_err(|e| format!("Failed to load manifest '{}': {e}", args.manifest.display()))?
    } else {
        let policy = command_schema_db::QualityPolicyFingerprint {
            min_confidence: config.quality.min_confidence,
            min_coverage: config.quality.min_coverage,
            allow_low_quality: config.quality.allow_low_quality,
        };
        command_schema_db::Manifest::new(PACKAGE_VERSION.to_string(), policy)
    };

    // 3. Create output directory
    fs::create_dir_all(&args.output).map_err(|e| {
        format!(
            "Failed to create output directory '{}': {e}",
            args.output.display()
        )
    })?;

    // 4. Build work list: determine which commands need extraction
    let current_policy = command_schema_db::QualityPolicyFingerprint {
        min_confidence: config.quality.min_confidence,
        min_coverage: config.quality.min_coverage,
        allow_low_quality: config.quality.allow_low_quality,
    };

    struct CommandWork {
        command: String,
        reason: String,
    }

    let mut to_extract: Vec<CommandWork> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    for cmd in &config.allowlist {
        // Check if excluded
        if config.is_excluded(cmd) {
            skipped.push(cmd.clone());
            continue;
        }

        // Check if installed (when required)
        if config.extraction.installed_only
            && !command_schema_discovery::extractor::command_exists(cmd)
        {
            skipped.push(cmd.clone());
            continue;
        }

        // Force flag bypasses all checks
        if args.force {
            to_extract.push(CommandWork {
                command: cmd.clone(),
                reason: "forced".to_string(),
            });
            continue;
        }

        // Probe current version and executable metadata
        let version = probe_command_version(cmd);
        let exe_path = resolve_executable_path(cmd);
        let (mtime, size) = exe_path
            .as_ref()
            .and_then(|p| fs::metadata(p).ok())
            .map(|m| {
                let mtime = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64);
                let size = m.len();
                (mtime, Some(size))
            })
            .unwrap_or((None, None));

        match manifest.get(cmd) {
            None => {
                // Command not in manifest — extract it
                to_extract.push(CommandWork {
                    command: cmd.clone(),
                    reason: "new".to_string(),
                });
            }
            Some(existing) => {
                // Check version change
                if version.is_some() && version != existing.version {
                    to_extract.push(CommandWork {
                        command: cmd.clone(),
                        reason: format!(
                            "version changed: {} -> {}",
                            existing.version.as_deref().unwrap_or("unknown"),
                            version.as_deref().unwrap_or("unknown")
                        ),
                    });
                    continue;
                }

                // Check fingerprint change when no version available
                if version.is_none() {
                    let path_str = exe_path.as_ref().map(|p| p.to_string_lossy().to_string());
                    let fingerprint_changed = path_str != existing.executable_path
                        || mtime != existing.mtime_secs
                        || size != existing.size_bytes;

                    if fingerprint_changed {
                        to_extract.push(CommandWork {
                            command: cmd.clone(),
                            reason: "fingerprint changed".to_string(),
                        });
                        continue;
                    }
                }

                // Check quality policy change
                let policy_changed = current_policy.min_confidence
                    != manifest.quality_policy.min_confidence
                    || current_policy.min_coverage != manifest.quality_policy.min_coverage
                    || current_policy.allow_low_quality
                        != manifest.quality_policy.allow_low_quality;

                if policy_changed {
                    to_extract.push(CommandWork {
                        command: cmd.clone(),
                        reason: "quality policy changed".to_string(),
                    });
                    continue;
                }

                // Check schema file integrity (missing or checksum mismatch)
                let schema_file = existing
                    .schema_file
                    .clone()
                    .unwrap_or_else(|| format!("{cmd}.json"));
                let schema_path = args.output.join(schema_file);
                let checksum_mismatch =
                    match command_schema_db::Manifest::calculate_checksum(&schema_path) {
                        Ok(current_checksum) => current_checksum != existing.checksum,
                        Err(_) => true, // file missing or unreadable
                    };

                if checksum_mismatch {
                    to_extract.push(CommandWork {
                        command: cmd.clone(),
                        reason: "schema file missing or checksum changed".to_string(),
                    });
                    continue;
                }

                // No changes detected — skip
                skipped.push(cmd.clone());
            }
        }
    }

    // 5. Extract marked commands in parallel using rayon
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(config.extraction.jobs)
        .build()
        .map_err(|e| format!("Failed to create thread pool: {e}"))?;

    let min_confidence = config.quality.min_confidence;
    let min_coverage = config.quality.min_coverage;
    let allow_low_quality = config.quality.allow_low_quality;
    let output_dir = &args.output;

    struct ExtractionOutcome {
        command: String,
        reason: String,
        schema_file: Option<String>,
        implementation: Option<String>,
        success: bool,
        error: Option<String>,
    }

    let outcomes: Vec<ExtractionOutcome> = pool.install(|| {
        to_extract
            .par_iter()
            .map(|work| {
                let policy = ExtractionQualityPolicy {
                    min_confidence,
                    min_coverage,
                    allow_low_quality,
                };
                let run =
                    command_schema_discovery::extractor::extract_command_schema_with_report_and_policy(
                        &work.command,
                        policy,
                    );

                match run.result.schema {
                    Some(ref schema) => {
                        let stem = schema_output_stem(schema, Some(&run.report));
                        let schema_file = format!("{stem}.json");
                        let path = output_dir.join(&schema_file);
                        match serde_json::to_string_pretty(schema) {
                            Ok(json) => {
                                if let Err(e) = fs::write(&path, &json) {
                                    return ExtractionOutcome {
                                        command: work.command.clone(),
                                        reason: work.reason.clone(),
                                        schema_file: None,
                                        implementation: run
                                            .report
                                            .resolved_implementation
                                            .clone(),
                                        success: false,
                                        error: Some(format!("Failed to write schema: {e}")),
                                    };
                                }
                                ExtractionOutcome {
                                    command: work.command.clone(),
                                    reason: work.reason.clone(),
                                    schema_file: Some(schema_file),
                                    implementation: run
                                        .report
                                        .resolved_implementation
                                        .clone(),
                                    success: true,
                                    error: None,
                                }
                            }
                            Err(e) => ExtractionOutcome {
                                command: work.command.clone(),
                                reason: work.reason.clone(),
                                schema_file: None,
                                implementation: run.report.resolved_implementation.clone(),
                                success: false,
                                error: Some(format!("Serialization failed: {e}")),
                            },
                        }
                    }
                    None => ExtractionOutcome {
                        command: work.command.clone(),
                        reason: work.reason.clone(),
                        schema_file: None,
                        implementation: run.report.resolved_implementation.clone(),
                        success: false,
                        error: Some("Extraction produced no schema".to_string()),
                    },
                }
            })
            .collect()
    });

    // 6. Update manifest with successful extractions
    for outcome in &outcomes {
        if outcome.success {
            let schema_file = outcome
                .schema_file
                .clone()
                .unwrap_or_else(|| format!("{}.json", outcome.command));
            let schema_path = args.output.join(&schema_file);
            let checksum =
                command_schema_db::Manifest::calculate_checksum(&schema_path).map_err(|e| {
                    format!(
                        "Failed to calculate checksum for '{}': {e}",
                        outcome.command
                    )
                })?;

            let version = probe_command_version(&outcome.command);
            let exe_path = resolve_executable_path(&outcome.command);
            let (mtime, size) = exe_path
                .as_ref()
                .and_then(|p| fs::metadata(p).ok())
                .map(|m| {
                    let mt = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64);
                    let sz = m.len();
                    (mt, Some(sz))
                })
                .unwrap_or((None, None));

            let metadata = command_schema_db::CommandMetadata {
                version,
                executable_path: exe_path.map(|p| p.to_string_lossy().to_string()),
                mtime_secs: mtime,
                size_bytes: size,
                extracted_at: chrono::Utc::now().to_rfc3339(),
                quality_tier: "high".to_string(),
                checksum,
                implementation: outcome.implementation.clone(),
                schema_file: Some(schema_file),
            };
            manifest.update_entry(outcome.command.clone(), metadata);
        }
    }

    // 7. Persist updated quality policy and tool version in manifest
    manifest.quality_policy = current_policy;
    if manifest.tool_version != PACKAGE_VERSION {
        manifest.tool_version = PACKAGE_VERSION.to_string();
    }

    // 8. Save updated manifest
    manifest
        .save(&args.manifest)
        .map_err(|e| format!("Failed to save manifest '{}': {e}", args.manifest.display()))?;

    // 8. Print summary report
    let extracted_count = outcomes.iter().filter(|o| o.success).count();
    let failed_count = outcomes.iter().filter(|o| !o.success).count();

    println!("CI Extract Summary:");
    println!("  Total commands: {}", config.allowlist.len());
    println!("  Extracted: {extracted_count} (new + updated)");
    println!("  Skipped: {} (unchanged)", skipped.len());
    println!("  Failed: {failed_count}");

    if outcomes.iter().any(|o| o.success) {
        println!("\nChanged commands:");
        for outcome in &outcomes {
            if outcome.success {
                println!("  {} ({})", outcome.command, outcome.reason);
            }
        }
    }

    if failed_count > 0 {
        eprintln!("\nFailures:");
        for outcome in &outcomes {
            if let Some(ref err) = outcome.error {
                eprintln!("  {}: {err}", outcome.command);
            }
        }
    }

    Ok(())
}

/// Probe a command's version by running `command --version` and parsing the output.
fn probe_command_version(command: &str) -> Option<String> {
    use std::process::Command as ProcessCommand;
    let output = ProcessCommand::new(command)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    command_schema_discovery::version::extract_version(&combined, command)
}

/// Resolve the absolute path of a command using `which`.
fn resolve_executable_path(command: &str) -> Option<PathBuf> {
    use std::process::Command as ProcessCommand;
    let output = ProcessCommand::new("which").arg(command).output().ok()?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path.is_empty() {
            None
        } else {
            Some(PathBuf::from(path))
        }
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// migrate command
// ---------------------------------------------------------------------------

fn run_migrate(args: MigrateArgs) -> Result<(), String> {
    match args.operation {
        MigrateOperation::Up(a) => run_migrate_up(a),
        MigrateOperation::Down(a) => run_migrate_down(a),
        MigrateOperation::Seed(a) => run_migrate_seed(a),
        MigrateOperation::Refresh(a) => run_migrate_refresh(a),
        MigrateOperation::Status(a) => run_migrate_status(a),
    }
}

fn run_migrate_up(args: MigrateUpArgs) -> Result<(), String> {
    let conn = rusqlite::Connection::open(&args.db)
        .map_err(|e| format!("Failed to open database '{}': {e}", args.db.display()))?;
    let mut migration = command_schema_sqlite::Migration::new(conn, &args.prefix)
        .map_err(|e| format!("Failed to initialize migration: {e}"))?;
    migration
        .up()
        .map_err(|e| format!("Migration up failed: {e}"))?;
    println!(
        "Migration up complete. Tables created with prefix '{}' in '{}'.",
        args.prefix,
        args.db.display()
    );
    Ok(())
}

fn run_migrate_down(args: MigrateDownArgs) -> Result<(), String> {
    let conn = rusqlite::Connection::open(&args.db)
        .map_err(|e| format!("Failed to open database '{}': {e}", args.db.display()))?;
    let mut migration = command_schema_sqlite::Migration::new(conn, &args.prefix)
        .map_err(|e| format!("Failed to initialize migration: {e}"))?;
    migration
        .down()
        .map_err(|e| format!("Migration down failed: {e}"))?;
    println!(
        "Migration down complete. Tables with prefix '{}' dropped from '{}'.",
        args.prefix,
        args.db.display()
    );
    Ok(())
}

fn run_migrate_seed(args: MigrateSeedArgs) -> Result<(), String> {
    let conn = rusqlite::Connection::open(&args.db)
        .map_err(|e| format!("Failed to open database '{}': {e}", args.db.display()))?;
    let mut migration = command_schema_sqlite::Migration::new(conn, &args.prefix)
        .map_err(|e| format!("Failed to initialize migration: {e}"))?;
    let report = migration
        .seed(&args.source)
        .map_err(|e| format!("Seed failed: {e}"))?;
    println!("Seed complete:");
    println!("  Commands inserted: {}", report.commands_inserted);
    println!("  Flags inserted: {}", report.flags_inserted);
    println!("  Subcommands inserted: {}", report.subcommands_inserted);
    println!("  Args inserted: {}", report.args_inserted);
    Ok(())
}

fn run_migrate_refresh(args: MigrateRefreshArgs) -> Result<(), String> {
    let conn = rusqlite::Connection::open(&args.db)
        .map_err(|e| format!("Failed to open database '{}': {e}", args.db.display()))?;
    let mut migration = command_schema_sqlite::Migration::new(conn, &args.prefix)
        .map_err(|e| format!("Failed to initialize migration: {e}"))?;
    let report = migration
        .refresh(&args.source)
        .map_err(|e| format!("Refresh failed: {e}"))?;
    println!("Refresh complete (tables dropped, recreated, and reseeded):");
    println!("  Commands inserted: {}", report.commands_inserted);
    println!("  Flags inserted: {}", report.flags_inserted);
    println!("  Subcommands inserted: {}", report.subcommands_inserted);
    println!("  Args inserted: {}", report.args_inserted);
    Ok(())
}

fn run_migrate_status(args: MigrateStatusArgs) -> Result<(), String> {
    let conn = rusqlite::Connection::open(&args.db)
        .map_err(|e| format!("Failed to open database '{}': {e}", args.db.display()))?;
    let migration = command_schema_sqlite::Migration::new(conn, &args.prefix)
        .map_err(|e| format!("Failed to initialize migration: {e}"))?;
    let status = migration
        .status()
        .map_err(|e| format!("Failed to get migration status: {e}"))?;
    println!("Migration Status:");
    println!(
        "  Tables exist: {}",
        if status.tables_exist { "yes" } else { "no" }
    );
    println!("  Command count: {}", status.command_count);
    println!("  Flag count: {}", status.flag_count);
    println!("  Subcommand count: {}", status.subcommand_count);
    println!("  Arg count: {}", status.arg_count);
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns the file extension for the given output format.
fn format_extension(format: command_schema_discovery::output::OutputFormat) -> &'static str {
    use command_schema_discovery::output::OutputFormat;
    match format {
        OutputFormat::Json => "json",
        OutputFormat::Yaml => "yaml",
        OutputFormat::Markdown => "md",
        OutputFormat::Table => "txt",
    }
}

/// Formats an `ExtractionReportBundle` in the requested output format.
fn format_report_bundle(
    bundle: &command_schema_discovery::report::ExtractionReportBundle,
    format: command_schema_discovery::output::OutputFormat,
) -> Result<String, String> {
    use command_schema_discovery::output::OutputFormat;
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(bundle)
            .map_err(|e| format!("JSON serialization failed: {e}")),
        OutputFormat::Yaml => {
            serde_yaml::to_string(bundle).map_err(|e| format!("YAML serialization failed: {e}"))
        }
        OutputFormat::Markdown | OutputFormat::Table => {
            let mut out = String::new();
            for report in &bundle.reports {
                out.push_str(&command_schema_discovery::output::format_report(
                    report, format,
                )?);
            }
            Ok(out)
        }
    }
}

fn parse_csv_list(raw: Option<String>) -> Vec<String> {
    raw.map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    })
    .unwrap_or_default()
}

fn schema_output_stem(
    schema: &command_schema_core::CommandSchema,
    report: Option<&command_schema_discovery::report::ExtractionReport>,
) -> String {
    let command = sanitize_filename_segment(&schema.command);
    let Some(implementation) = report.and_then(|r| r.resolved_implementation.as_deref()) else {
        return command;
    };
    let implementation = sanitize_filename_segment(implementation);
    if implementation.is_empty() || implementation.eq_ignore_ascii_case(&command) {
        return command;
    }
    format!("{command}__{implementation}")
}

fn sanitize_filename_segment(raw: &str) -> String {
    fn symbolic_alias(raw: &str) -> Option<&'static str> {
        match raw {
            "[" => Some("lbracket"),
            "]" => Some("rbracket"),
            _ => None,
        }
    }

    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let cleaned = out.trim_matches('-');
    if cleaned.is_empty() {
        if let Some(alias) = symbolic_alias(raw) {
            alias.to_string()
        } else {
            let mut hex = String::new();
            for byte in raw.as_bytes() {
                use std::fmt::Write as _;
                let _ = write!(&mut hex, "{byte:02x}");
            }
            if hex.is_empty() {
                "unknown".to_string()
            } else {
                format!("cmd-{hex}")
            }
        }
    } else {
        cleaned.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_csv_list, sanitize_filename_segment};

    #[test]
    fn test_parse_csv_list_trims_and_drops_empty() {
        let parsed = parse_csv_list(Some(" git, docker, ,cargo ".to_string()));
        assert_eq!(parsed, vec!["git", "docker", "cargo"]);
    }

    #[test]
    fn test_parse_csv_list_none_is_empty() {
        let parsed = parse_csv_list(None);
        assert!(parsed.is_empty());
    }

    #[test]
    fn test_sanitize_filename_segment_keeps_safe_chars() {
        assert_eq!(sanitize_filename_segment("awk"), "awk");
        assert_eq!(sanitize_filename_segment("gawk-5.3"), "gawk-5.3");
        assert_eq!(sanitize_filename_segment("awk (gnu)"), "awk--gnu");
    }

    #[test]
    fn test_sanitize_filename_segment_symbolic_command_aliases() {
        assert_eq!(sanitize_filename_segment("["), "lbracket");
        assert_eq!(sanitize_filename_segment("]"), "rbracket");
    }
}
