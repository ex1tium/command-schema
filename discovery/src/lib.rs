//! Offline command schema discovery and parsing.
//!
//! This crate provides tools for extracting structured [`CommandSchema`]s from
//! CLI help output. It supports multiple help-text formats (GNU, Clap, Cobra,
//! Argparse, NPM-style, BSD, and generic section-based) and can optionally
//! probe commands to discover subcommands recursively.
//!
//! # Main entry points
//!
//! - [`parse_help_text`] — parse pre-captured help text without running any
//!   commands.
//! - [`parse_help_text_with_report`] — same, but with full diagnostics and
//!   quality policy gating.
//! - [`extractor::extract_command_schema`] — probe a command's `--help` output
//!   and extract a schema (requires the command to be installed).
//!
//! # Example
//!
//! ```
//! use command_schema_discovery::parse_help_text;
//!
//! let help = "\
//! Usage: mycli [OPTIONS] <FILE>
//!
//! Arguments:
//!   <FILE>  Input file to process
//!
//! Options:
//!   -v, --verbose    Enable verbose output
//!   -o, --output <PATH>  Output file
//!   -h, --help       Print help
//! ";
//!
//! let result = parse_help_text("mycli", help);
//! assert!(result.success);
//! let schema = result.schema.unwrap();
//! assert_eq!(schema.command, "mycli");
//! assert!(schema.global_flags.iter().any(|f| f.long.as_deref() == Some("--verbose")));
//! ```
//!
//! # Crate type
//!
//! This is a **library-only crate** with no binary targets. For CLI usage, use
//! the `command-schema-cli` crate which provides the `schema-discover` binary.
//!
//! All functionality is available through library APIs:
//! - [`parse_help_text`] — offline parsing, no command execution required
//! - [`parse_help_text_with_report`] — offline parsing with quality diagnostics
//! - [`extractor::extract_command_schema`] — live extraction (runs the command)
//!
//! [`CommandSchema`]: command_schema_core::CommandSchema

pub mod cache;
pub mod discover;
pub mod extractor;
pub mod output;
pub mod parser;
pub mod report;
pub mod version;

use command_schema_core::ExtractionResult;
use extractor::{ExtractionQualityPolicy, ExtractionRun};
use parser::HelpParser;
use report::{ExtractionReport, FailureCode, QualityTier};

/// Parses pre-captured help text into a schema without executing any commands.
///
/// This is the primary entry point for offline parsing. Pass the command name
/// and its `--help` output, and receive an [`ExtractionResult`] with the
/// parsed schema, detected format, and any warnings.
///
/// # Examples
///
/// ```
/// use command_schema_discovery::parse_help_text;
///
/// let help = "\
/// Usage: ls [OPTION]... [FILE]...
///
///   -a, --all         do not ignore entries starting with .
///   -l                use a long listing format
///   -h, --human-readable  print sizes in human readable format
/// ";
///
/// let result = parse_help_text("ls", help);
/// if let Some(schema) = &result.schema {
///     for flag in &schema.global_flags {
///         println!("{}", flag.canonical_name());
///     }
/// }
/// ```
pub fn parse_help_text(command: &str, help_text: &str) -> ExtractionResult {
    let mut parser = HelpParser::new(command, help_text);
    let schema = parser.parse().map(|mut s| {
        s.schema_version = Some(command_schema_core::SCHEMA_CONTRACT_VERSION.to_string());
        s
    });
    let warnings = parser.warnings().to_vec();
    let detected_format = parser.detected_format();
    let success = schema.is_some();

    ExtractionResult {
        schema,
        raw_output: help_text.to_string(),
        detected_format,
        warnings,
        success,
    }
}

/// Parses pre-captured help text with full reporting and quality policy gating.
///
/// Like [`parse_help_text`], but additionally produces an
/// [`ExtractionReport`] with coverage metrics,
/// quality tier classification, and applies the given
/// [`ExtractionQualityPolicy`] to determine acceptance.
///
/// # Examples
///
/// ```
/// use command_schema_discovery::{parse_help_text_with_report, extractor::ExtractionQualityPolicy};
///
/// let help = "\
/// Usage: tool [OPTIONS]
///
///   -v, --verbose  Verbose output
///   -q, --quiet    Suppress output
/// ";
///
/// let run = parse_help_text_with_report("tool", help, ExtractionQualityPolicy::default());
/// println!("Quality tier: {:?}", run.report.quality_tier);
/// println!("Confidence: {:.2}", run.report.confidence);
/// ```
pub fn parse_help_text_with_report(
    command: &str,
    help_text: &str,
    policy: ExtractionQualityPolicy,
) -> ExtractionRun {
    let mut parser = HelpParser::new(command, help_text);
    let schema = parser.parse().map(|mut s| {
        s.schema_version = Some(command_schema_core::SCHEMA_CONTRACT_VERSION.to_string());
        s
    });
    let warnings = parser.warnings().to_vec();
    let detected_format = parser.detected_format();
    let diagnostics = parser.diagnostics().clone();

    let (success, failure_code, failure_detail) = match &schema {
        Some(s) => {
            let has_entities = !s.global_flags.is_empty()
                || !s.subcommands.is_empty()
                || !s.positional.is_empty();
            if has_entities {
                (true, None, None)
            } else {
                (
                    false,
                    Some(FailureCode::ParseFailed),
                    Some("Parsed schema contains no entities".to_string()),
                )
            }
        }
        None => (
            false,
            Some(FailureCode::ParseFailed),
            Some("Help text parsing produced no schema".to_string()),
        ),
    };

    let confidence = schema.as_ref().map_or(0.0, |s| s.confidence);

    let run = ExtractionRun {
        result: ExtractionResult {
            schema,
            raw_output: help_text.to_string(),
            detected_format,
            warnings: warnings.clone(),
            success,
        },
        report: ExtractionReport {
            command: command.to_string(),
            success,
            accepted_for_suggestions: false,
            quality_tier: QualityTier::Failed,
            quality_reasons: Vec::new(),
            failure_code,
            failure_detail,
            selected_format: detected_format.map(extractor::help_format_label),
            format_scores: extractor::to_format_score_reports(&diagnostics.format_scores),
            confidence,
            coverage: diagnostics.coverage(),
            relevant_lines: diagnostics.relevant_lines,
            recognized_lines: diagnostics.recognized_lines,
            unresolved_lines: diagnostics.unresolved_lines.clone(),
            parsers_used: diagnostics.parsers_used,
            probe_attempts: Vec::new(),
            warnings,
            validation_errors: Vec::new(),
        },
    };

    extractor::apply_quality_policy(run, policy)
}
