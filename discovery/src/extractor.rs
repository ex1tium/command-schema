//! Command schema extraction via help probing.
//!
//! Automatically extracts command schemas by probing man pages first, then
//! running `--help` commands and recursively probing subcommands with cycle
//! detection and a bounded probe budget. The extractor tries multiple help
//! flags (`--help`, `-h`, `-?`) and conditionally tries `help` when probe
//! output explicitly suggests it.
//!
//! # Quality policy
//!
//! The [`ExtractionQualityPolicy`] controls acceptance thresholds:
//! - `min_confidence` — minimum parser confidence score (0.0–1.0)
//! - `min_coverage` — minimum ratio of recognized help lines (0.0–1.0)
//! - `allow_low_quality` — whether to emit schemas below thresholds
//!
//! # Example
//!
//! ```no_run
//! use command_schema_discovery::extractor::{extract_command_schema, ExtractionQualityPolicy};
//!
//! let result = extract_command_schema("cargo");
//! if let Some(schema) = result.schema {
//!     println!("{} has {} subcommands", schema.command, schema.subcommands.len());
//! }
//! ```

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

use super::parser::util::looks_like_man_title_line;
use super::parser::{FormatScore, HelpParser};
use super::report::{
    ExtractionReport, FailureCode, FormatScoreReport, ProbeAttemptReport, QualityTier,
};
use command_schema_core::{ExtractionResult, HelpFormat, SubcommandSchema, validate_schema};

/// Maximum number of unique command invocations to probe recursively.
///
/// Depth is intentionally unbounded; this budget is the safety guard.
const MAX_RECURSIVE_PROBE_BUDGET: usize = 4096;

/// Timeout for help commands (milliseconds).
const HELP_TIMEOUT_MS: u64 = 5000;
static PROBE_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Help flags to try in order.
const HELP_FLAGS: &[&str] = &["--help", "-h", "-?"];

/// Suggested quality threshold for confidence in production runs.
pub const DEFAULT_MIN_CONFIDENCE: f64 = 0.6;

/// Suggested quality threshold for parser coverage in production runs.
pub const DEFAULT_MIN_COVERAGE: f64 = 0.2;

/// Policy controlling extraction quality acceptance.
///
/// Configures the minimum confidence and coverage thresholds for a schema
/// to be accepted. Use [`Default::default()`] for production-grade defaults
/// or [`permissive()`](Self::permissive) for development/testing.
///
/// # Examples
///
/// ```
/// use command_schema_discovery::extractor::ExtractionQualityPolicy;
///
/// // Production defaults
/// let policy = ExtractionQualityPolicy::default();
/// assert_eq!(policy.min_confidence, 0.6);
/// assert_eq!(policy.min_coverage, 0.2);
/// assert!(!policy.allow_low_quality);
///
/// // Accept everything (for testing)
/// let permissive = ExtractionQualityPolicy::permissive();
/// assert_eq!(permissive.min_confidence, 0.0);
/// assert!(permissive.allow_low_quality);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ExtractionQualityPolicy {
    pub min_confidence: f64,
    pub min_coverage: f64,
    pub allow_low_quality: bool,
}

impl Default for ExtractionQualityPolicy {
    fn default() -> Self {
        Self {
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            min_coverage: DEFAULT_MIN_COVERAGE,
            allow_low_quality: false,
        }
    }
}

impl ExtractionQualityPolicy {
    pub fn permissive() -> Self {
        Self {
            min_confidence: 0.0,
            min_coverage: 0.0,
            allow_low_quality: true,
        }
    }
}

/// Extraction output with both schema result and diagnostics report.
///
/// Combines the parsed [`ExtractionResult`] with a detailed
/// [`ExtractionReport`] that includes coverage metrics, quality tier,
/// and probe attempt history.
pub struct ExtractionRun {
    pub result: ExtractionResult,
    pub report: ExtractionReport,
}

#[derive(Debug, Clone)]
struct ProbeAttempt {
    help_flag: String,
    argv: Vec<String>,
    exit_code: Option<i32>,
    timed_out: bool,
    error: Option<String>,
    rejection_reason: Option<String>,
    failure_code: Option<FailureCode>,
    output_source: Option<String>,
    output_len: usize,
    output_preview: Option<String>,
    accepted: bool,
}

impl ProbeAttempt {
    fn new(help_flag: &str, argv: Vec<String>) -> Self {
        Self {
            help_flag: help_flag.to_string(),
            argv,
            exit_code: None,
            timed_out: false,
            error: None,
            rejection_reason: None,
            failure_code: None,
            output_source: None,
            output_len: 0,
            output_preview: None,
            accepted: false,
        }
    }
}

#[derive(Debug, Clone)]
struct ProbeRun {
    help_output: Option<String>,
    attempts: Vec<ProbeAttempt>,
}

#[derive(Debug)]
struct DirectProbeOutcome {
    attempt: ProbeAttempt,
    accepted_output: Option<String>,
    spawn_not_found: bool,
}

struct ProbeWorkspace {
    path: PathBuf,
}

impl ProbeWorkspace {
    fn create() -> Option<Self> {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let seq = PROBE_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        path.push(format!(
            "command-schema-probe-{}-{nanos}-{seq}",
            std::process::id()
        ));
        if fs::create_dir(&path).is_ok() {
            Some(Self { path })
        } else {
            None
        }
    }
}

impl Drop for ProbeWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Resolved identity of a command binary.
///
/// Both fields store only the **basename** (not a full path) to avoid leaking
/// absolute filesystem paths into serialized reports and manifests.
#[derive(Debug, Clone, Default)]
struct CommandIdentity {
    /// Basename of the resolved executable (e.g. `gawk` for `awk`).
    resolved_executable_path: Option<String>,
    /// Resolved implementation name, typically identical to the executable basename.
    resolved_implementation: Option<String>,
}

/// Probes a command's help output and returns the raw text.
pub fn probe_command_help(command: &str) -> Option<String> {
    probe_command_help_with_metadata(command).help_output
}

fn probe_command_help_with_metadata(command: &str) -> ProbeRun {
    // Split command into parts (e.g., "git remote" -> ["git", "remote"])
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return ProbeRun {
            help_output: None,
            attempts: Vec::new(),
        };
    }

    let mut attempts = Vec::with_capacity(HELP_FLAGS.len());
    let base_command = parts[0].to_ascii_lowercase();
    let mut env_overrides = default_probe_env();
    env_overrides.extend(command_specific_probe_env(base_command.as_str()));

    for man_page in man_probe_pages(&parts) {
        let cmd_parts = vec!["man".to_string(), man_page.clone()];
        let label = format!("man:{man_page}");

        debug!(command = ?cmd_parts, "Probing man page");
        let outcome = try_direct_probe(&cmd_parts, &label, &env_overrides);
        if outcome.spawn_not_found {
            // If `man` itself is unavailable there is no value in trying
            // additional man pages for this command chain.
            break;
        }
        attempts.push(outcome.attempt);
        if let Some(help_text) = outcome.accepted_output {
            return ProbeRun {
                help_output: Some(adapt_help_output_for_command(command, help_text)),
                attempts,
            };
        }
    }

    for suffix in command_specific_probe_suffixes(base_command.as_str()) {
        let mut cmd_parts: Vec<String> = parts.iter().map(|part| (*part).to_string()).collect();
        cmd_parts.extend(suffix.iter().map(|part| (*part).to_string()));
        let label = suffix.join(" ");

        debug!(command = ?cmd_parts, "Probing command-specific help");
        let outcome = try_direct_probe(&cmd_parts, &label, &env_overrides);
        if outcome.spawn_not_found {
            continue;
        }
        attempts.push(outcome.attempt);
        if let Some(help_text) = outcome.accepted_output {
            return ProbeRun {
                help_output: Some(adapt_help_output_for_command(command, help_text)),
                attempts,
            };
        }
    }

    for help_flag in HELP_FLAGS {
        let mut cmd_parts: Vec<String> = parts.iter().map(|part| (*part).to_string()).collect();
        cmd_parts.push((*help_flag).to_string());

        debug!(command = ?cmd_parts, "Probing help");
        let outcome = try_direct_probe(&cmd_parts, help_flag, &env_overrides);
        if outcome.spawn_not_found {
            // Shell builtin fallback (e.g. `cd`) for commands that don't exist
            // as standalone executables.
            debug!(
                command = ?cmd_parts,
                "Direct spawn failed, trying shell fallback probe"
            );
            let shell_probe = probe_shell_help(&parts, help_flag, &env_overrides);
            attempts.push(shell_probe.attempt);
            if let Some(help_text) = shell_probe.accepted_output {
                return ProbeRun {
                    help_output: Some(adapt_help_output_for_command(command, help_text)),
                    attempts,
                };
            }
            continue;
        }

        attempts.push(outcome.attempt);
        if let Some(help_text) = outcome.accepted_output {
            debug!(
                command = command,
                help_flag = help_flag,
                length = help_text.len(),
                "Got help output"
            );
            return ProbeRun {
                help_output: Some(adapt_help_output_for_command(command, help_text)),
                attempts,
            };
        }
    }

    if should_probe_help_subcommand(&parts, &attempts) {
        let mut cmd_parts: Vec<String> = parts.iter().map(|part| (*part).to_string()).collect();
        if parts.len() > 1 {
            // For "git remote", try "git help remote".
            cmd_parts.insert(1, "help".to_string());
        } else {
            cmd_parts.push("help".to_string());
        }

        debug!(command = ?cmd_parts, "Probing help subcommand");
        let outcome = try_direct_probe(&cmd_parts, "help", &env_overrides);
        if outcome.spawn_not_found {
            let shell_probe = probe_shell_help(&parts, "help", &env_overrides);
            attempts.push(shell_probe.attempt);
            if let Some(help_text) = shell_probe.accepted_output {
                return ProbeRun {
                    help_output: Some(adapt_help_output_for_command(command, help_text)),
                    attempts,
                };
            }
        } else {
            attempts.push(outcome.attempt);
            if let Some(help_text) = outcome.accepted_output {
                return ProbeRun {
                    help_output: Some(adapt_help_output_for_command(command, help_text)),
                    attempts,
                };
            }
        }
    }

    ProbeRun {
        help_output: None,
        attempts,
    }
}

fn should_probe_help_subcommand(parts: &[&str], attempts: &[ProbeAttempt]) -> bool {
    if parts.len() > 1 {
        return true;
    }
    let base = parts
        .first()
        .copied()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if base.is_empty() {
        return false;
    }

    attempts.iter().any(|attempt| {
        if attempt.accepted {
            return false;
        }
        let Some(preview) = attempt.output_preview.as_deref() else {
            return false;
        };
        let lower = preview.to_ascii_lowercase();
        (lower.contains("try") || lower.contains("use"))
            && lower.contains(" help")
            && lower.contains(&base)
    })
}

fn command_specific_probe_suffixes(base_command: &str) -> Vec<Vec<&'static str>> {
    match base_command {
        "ps" => vec![
            vec!["--help", "all"],
            vec!["--help", "simple"],
            vec!["--help", "list"],
            vec!["--help", "output"],
        ],
        _ => Vec::new(),
    }
}

fn man_probe_pages(parts: &[&str]) -> Vec<String> {
    if parts.is_empty() {
        return Vec::new();
    }

    if !parts.iter().all(|part| is_plausible_man_token(part)) {
        return Vec::new();
    }

    let mut pages = Vec::new();
    let mut seen = HashSet::new();
    if parts.len() == 1 {
        push_unique_man_page(&mut pages, &mut seen, parts[0].to_string());
        return pages;
    }

    // For nested commands (e.g. `git remote add`) try the most specific page
    // first and then progressively less specific parent pages.
    for depth in (2..=parts.len()).rev() {
        let suffix = parts[1..depth].join("-");
        push_unique_man_page(&mut pages, &mut seen, format!("{}-{suffix}", parts[0]));
    }

    pages
}

fn push_unique_man_page(pages: &mut Vec<String>, seen: &mut HashSet<String>, page: String) {
    let trimmed = page.trim();
    if trimmed.is_empty() {
        return;
    }
    if seen.insert(trimmed.to_string()) {
        pages.push(trimmed.to_string());
    }
}

fn is_plausible_man_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '_' | '-'))
}

fn command_specific_probe_env(base_command: &str) -> Vec<(&'static str, &'static str)> {
    match base_command {
        // Some ansible installations fail to resolve writable temp dirs in
        // constrained environments unless this is explicitly overridden.
        "ansible" => vec![
            ("ANSIBLE_LOCAL_TEMP", "/tmp"),
            ("ANSIBLE_REMOTE_TEMP", "/tmp"),
        ],
        _ => Vec::new(),
    }
}

fn default_probe_env() -> Vec<(&'static str, &'static str)> {
    vec![
        // Prevent graphical helpers from opening windows during probes.
        ("DISPLAY", ""),
        ("WAYLAND_DISPLAY", ""),
        ("BROWSER", "true"),
        // Keep interactive helpers from switching terminal modes.
        ("DEBIAN_FRONTEND", "noninteractive"),
        ("TERM", "dumb"),
        ("NO_COLOR", "1"),
        // Avoid interactive pagers when commands route help through pager tools.
        ("PAGER", "cat"),
        ("MANPAGER", "cat"),
        ("SYSTEMD_PAGER", "cat"),
        ("GIT_PAGER", "cat"),
    ]
}

fn adapt_help_output_for_command(command: &str, help_text: String) -> String {
    let base_command = command
        .split_whitespace()
        .next()
        .unwrap_or(command)
        .to_ascii_lowercase();
    match base_command.as_str() {
        "service" => adapt_service_help_output(&help_text),
        _ => help_text,
    }
}

fn adapt_service_help_output(help_text: &str) -> String {
    if !help_text.contains("Usage: service") {
        return help_text.to_string();
    }
    if help_text.contains("\nOptions:") || help_text.contains("\nFlags:") {
        return help_text.to_string();
    }

    let mut adapted = help_text.trim_end().to_string();
    adapted.push_str(
        "\n\nOptions:\n  --status-all    list all services and current status\n  --full-restart  run a full restart for a service\n\nArguments:\n  service_name    service to operate on\n  command         service command to execute\n",
    );
    adapted
}

fn try_direct_probe(
    cmd_parts: &[String],
    help_flag: &str,
    env_overrides: &[(&str, &str)],
) -> DirectProbeOutcome {
    let mut attempt = ProbeAttempt::new(help_flag, cmd_parts.to_vec());
    let mut command = Command::new(&cmd_parts[0]);
    let probe_workspace = ProbeWorkspace::create();
    command
        .args(&cmd_parts[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(workspace) = probe_workspace.as_ref() {
        command.current_dir(&workspace.path);
    }
    for (key, value) in env_overrides {
        command.env(key, value);
    }
    let spawn_result = command.spawn();

    match spawn_result {
        Ok(mut child) => {
            // Drain stdout and stderr in background threads to prevent
            // deadlock when the child's pipe buffer fills before it exits.
            let stdout_thread = child.stdout.take().map(|pipe| {
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let mut pipe = pipe;
                    let result = pipe.read_to_end(&mut buf);
                    (buf, result)
                })
            });
            let stderr_thread = child.stderr.take().map(|pipe| {
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let mut pipe = pipe;
                    let result = pipe.read_to_end(&mut buf);
                    (buf, result)
                })
            });

            let timeout = Duration::from_millis(HELP_TIMEOUT_MS);
            match wait_for_child_with_timeout(&mut child, timeout) {
                Ok(Some(status)) => {
                    attempt.exit_code = status.code();
                    let mut io_errors = Vec::new();

                    let stdout_buf = stdout_thread
                        .and_then(|t| t.join().ok())
                        .map(|(buf, res)| {
                            if let Err(e) = res {
                                debug!(command = ?cmd_parts, error = %e, "Failed to read stdout");
                                io_errors.push(format!("stdout read failed: {e}"));
                            }
                            buf
                        })
                        .unwrap_or_default();
                    let stderr_buf = stderr_thread
                        .and_then(|t| t.join().ok())
                        .map(|(buf, res)| {
                            if let Err(e) = res {
                                debug!(command = ?cmd_parts, error = %e, "Failed to read stderr");
                                io_errors.push(format!("stderr read failed: {e}"));
                            }
                            buf
                        })
                        .unwrap_or_default();
                    if !io_errors.is_empty() {
                        attempt.error = Some(io_errors.join("; "));
                    }

                    let stdout = String::from_utf8_lossy(&stdout_buf);
                    let stderr = String::from_utf8_lossy(&stderr_buf);
                    let (help_text, output_source) = if stdout.len() > stderr.len() {
                        (stdout.to_string(), "stdout")
                    } else {
                        (stderr.to_string(), "stderr")
                    };
                    attempt.output_source = Some(output_source.to_string());
                    attempt.output_len = help_text.len();
                    attempt.output_preview = output_preview(&help_text);

                    if is_help_output(&help_text) {
                        attempt.accepted = true;
                        return DirectProbeOutcome {
                            attempt,
                            accepted_output: Some(help_text),
                            spawn_not_found: false,
                        };
                    }

                    let classification = classify_rejection(&help_text);
                    attempt.rejection_reason = Some(classification.reason);
                    attempt.failure_code = Some(classification.failure_code);
                    DirectProbeOutcome {
                        attempt,
                        accepted_output: None,
                        spawn_not_found: false,
                    }
                }
                Ok(None) => {
                    attempt.timed_out = true;
                    attempt.failure_code = Some(FailureCode::Timeout);
                    debug!(
                        command = ?cmd_parts,
                        timeout_ms = HELP_TIMEOUT_MS,
                        "Help command timed out, killing process"
                    );
                    let _ = child.kill();
                    let _ = child.wait();
                    DirectProbeOutcome {
                        attempt,
                        accepted_output: None,
                        spawn_not_found: false,
                    }
                }
                Err(e) => {
                    attempt.error = Some(format!("wait failed: {e}"));
                    debug!(command = ?cmd_parts, error = %e, "Failed to wait on help command");
                    let _ = child.kill();
                    let _ = child.wait();
                    DirectProbeOutcome {
                        attempt,
                        accepted_output: None,
                        spawn_not_found: false,
                    }
                }
            }
        }
        Err(e) => {
            if e.kind() == ErrorKind::NotFound {
                attempt.failure_code = Some(FailureCode::NotInstalled);
                return DirectProbeOutcome {
                    attempt,
                    accepted_output: None,
                    spawn_not_found: true,
                };
            }
            if e.kind() == ErrorKind::PermissionDenied {
                attempt.failure_code = Some(FailureCode::PermissionBlocked);
            }
            attempt.error = Some(format!("spawn failed: {e}"));
            debug!(command = ?cmd_parts, error = %e, "Failed to spawn help command");
            DirectProbeOutcome {
                attempt,
                accepted_output: None,
                spawn_not_found: false,
            }
        }
    }
}

struct ShellProbeResult {
    attempt: ProbeAttempt,
    accepted_output: Option<String>,
}

/// Returns `true` if `s` contains shell metacharacters that could allow injection.
fn contains_shell_metacharacters(s: &str) -> bool {
    s.chars().any(|ch| {
        matches!(
            ch,
            '|' | ';'
                | '&'
                | '$'
                | '`'
                | '('
                | ')'
                | '{'
                | '}'
                | '<'
                | '>'
                | '!'
                | '\''
                | '"'
                | '\\'
                | '\n'
                | '\r'
        )
    })
}

fn probe_shell_help(
    parts: &[&str],
    help_flag: &str,
    env_overrides: &[(&str, &str)],
) -> ShellProbeResult {
    // Reject commands containing shell metacharacters to prevent injection
    // since we pass the joined string to `bash -lc`.
    if parts.iter().any(|p| contains_shell_metacharacters(p))
        || contains_shell_metacharacters(help_flag)
    {
        let argv = vec!["bash".to_string(), "-lc".to_string(), parts.join(" ")];
        let mut attempt = ProbeAttempt::new(help_flag, argv);
        attempt.error = Some("rejected: command contains shell metacharacters".to_string());
        return ShellProbeResult {
            attempt,
            accepted_output: None,
        };
    }

    let shell_cmd = if help_flag == "help" {
        format!("help {}", parts.join(" "))
    } else {
        format!("{} {}", parts.join(" "), help_flag)
    };

    let argv = vec!["bash".to_string(), "-lc".to_string(), shell_cmd.clone()];
    let mut attempt = ProbeAttempt::new(help_flag, argv);

    let mut command = Command::new("bash");
    let probe_workspace = ProbeWorkspace::create();
    command
        .arg("-lc")
        .arg(&shell_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(workspace) = probe_workspace.as_ref() {
        command.current_dir(&workspace.path);
    }
    for (key, value) in env_overrides {
        command.env(key, value);
    }
    let spawn = command.spawn();

    let Ok(mut child) = spawn else {
        let err = spawn.err().map_or_else(
            || "spawn failed".to_string(),
            |e| format!("shell spawn failed: {e}"),
        );
        attempt.error = Some(err);
        return ShellProbeResult {
            attempt,
            accepted_output: None,
        };
    };

    // Drain pipes concurrently to prevent deadlock when buffer fills.
    let stdout_thread = child.stdout.take().map(|pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut pipe = pipe;
            let result = pipe.read_to_end(&mut buf);
            (buf, result)
        })
    });
    let stderr_thread = child.stderr.take().map(|pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut pipe = pipe;
            let result = pipe.read_to_end(&mut buf);
            (buf, result)
        })
    });
    let timeout = Duration::from_millis(HELP_TIMEOUT_MS);

    match wait_for_child_with_timeout(&mut child, timeout) {
        Ok(Some(status)) => {
            attempt.exit_code = status.code();
            let mut io_errors = Vec::new();

            let stdout_buf = stdout_thread
                .and_then(|t| t.join().ok())
                .map(|(buf, res)| {
                    if let Err(e) = res {
                        io_errors.push(format!("stdout read failed: {e}"));
                    }
                    buf
                })
                .unwrap_or_default();
            let stderr_buf = stderr_thread
                .and_then(|t| t.join().ok())
                .map(|(buf, res)| {
                    if let Err(e) = res {
                        io_errors.push(format!("stderr read failed: {e}"));
                    }
                    buf
                })
                .unwrap_or_default();
            if !io_errors.is_empty() {
                attempt.error = Some(io_errors.join("; "));
            }

            let stdout = String::from_utf8_lossy(&stdout_buf);
            let stderr = String::from_utf8_lossy(&stderr_buf);
            let (help_text, output_source) = if stdout.len() > stderr.len() {
                (stdout.to_string(), "stdout")
            } else {
                (stderr.to_string(), "stderr")
            };

            attempt.output_source = Some(output_source.to_string());
            attempt.output_len = help_text.len();
            attempt.output_preview = output_preview(&help_text);
            if is_help_output(&help_text) {
                attempt.accepted = true;
                return ShellProbeResult {
                    attempt,
                    accepted_output: Some(help_text),
                };
            }
            let classification = classify_rejection(&help_text);
            attempt.rejection_reason = Some(classification.reason);
            attempt.failure_code = Some(classification.failure_code);

            ShellProbeResult {
                attempt,
                accepted_output: None,
            }
        }
        Ok(None) => {
            attempt.timed_out = true;
            attempt.failure_code = Some(FailureCode::Timeout);
            let _ = child.kill();
            let _ = child.wait();
            ShellProbeResult {
                attempt,
                accepted_output: None,
            }
        }
        Err(e) => {
            attempt.error = Some(format!("shell wait failed: {e}"));
            let _ = child.kill();
            let _ = child.wait();
            ShellProbeResult {
                attempt,
                accepted_output: None,
            }
        }
    }
}

fn wait_for_child_with_timeout(
    child: &mut Child,
    timeout: Duration,
) -> std::io::Result<Option<ExitStatus>> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if start.elapsed() >= timeout {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Checks if text looks like help output.
fn is_help_output(text: &str) -> bool {
    let trimmed = text.trim();
    let text_lower = trimmed.to_ascii_lowercase();

    // Must have some minimum content
    if trimmed.len() < 20 {
        return false;
    }

    let hard_fail_markers = [
        "command not found",
        "no such file or directory",
        "is not recognized as an internal or external command",
    ];
    if hard_fail_markers
        .iter()
        .any(|marker| text_lower.contains(marker))
    {
        return false;
    }

    let has_usage_line = trimmed
        .lines()
        .any(|line| line.trim_start().to_ascii_lowercase().starts_with("usage:"));
    let has_structured_sections = trimmed.lines().any(|line| {
        let normalized = line.trim_start().to_ascii_lowercase();
        normalized.starts_with("options:")
            || normalized.starts_with("flags:")
            || normalized.starts_with("commands:")
            || normalized.starts_with("arguments:")
            || normalized.starts_with("positional arguments:")
    });
    let leading_window = trimmed
        .lines()
        .take(3)
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();

    let option_error_markers = [
        "illegal option",
        "unknown option",
        "unknown argument",
        "invalid option",
        "unrecognized option",
    ];
    let suggestion_only_help_hint = !has_usage_line
        && !has_structured_sections
        && trimmed
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            <= 2
        && (text_lower.contains(" is unknown, try ")
            || text_lower.contains("unknown argument")
            || text_lower.contains("unknown option"))
        && text_lower.contains(" help");
    if suggestion_only_help_hint {
        return false;
    }
    if option_error_markers
        .iter()
        .any(|marker| leading_window.contains(marker))
        && !has_usage_line
        && !has_structured_sections
    {
        return false;
    }

    // Explicit usage line is a strong help indicator for compact outputs.
    if has_usage_line {
        return true;
    }

    if is_man_page_output(trimmed) {
        return true;
    }

    // Should contain help-like keywords
    let help_indicators = [
        "usage",
        "options",
        "commands",
        "flags",
        "arguments",
        "description",
    ];

    help_indicators.iter().any(|&ind| text_lower.contains(ind))
}

fn is_man_page_output(text: &str) -> bool {
    let mut title_lines = 0usize;
    let mut section_headers = 0usize;

    for line in text.lines().take(120) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if looks_like_man_title_line(trimmed) {
            title_lines += 1;
        }
        if matches!(
            trimmed,
            "NAME"
                | "SYNOPSIS"
                | "DESCRIPTION"
                | "OPTIONS"
                | "COMMANDS"
                | "EXAMPLES"
                | "SEE ALSO"
                | "FILES"
                | "ENVIRONMENT"
        ) {
            section_headers += 1;
        }
    }

    section_headers >= 2 || (title_lines >= 1 && section_headers >= 1)
}

fn output_preview(text: &str) -> Option<String> {
    const MAX_PREVIEW_LEN: usize = 160;

    let first_non_empty = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    if first_non_empty.is_empty() {
        return None;
    }
    if first_non_empty.chars().count() <= MAX_PREVIEW_LEN {
        return Some(first_non_empty.to_string());
    }
    let truncated = first_non_empty
        .chars()
        .take(MAX_PREVIEW_LEN)
        .collect::<String>();
    Some(format!("{truncated}..."))
}

struct RejectionClassification {
    reason: String,
    failure_code: FailureCode,
}

fn classify_rejection(help_text: &str) -> RejectionClassification {
    let lower = help_text.to_ascii_lowercase();

    let environment_blocked_markers = [
        "operation not permitted",
        "permission denied",
        "no new privileges",
        "cannot open audit interface",
        "unable to initialize netlink socket",
        "can't open display",
        "cannot open display",
        "error: can't open display",
    ];
    if environment_blocked_markers
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return RejectionClassification {
            reason: "environment-blocked".to_string(),
            failure_code: FailureCode::PermissionBlocked,
        };
    }

    let option_error_markers = [
        "illegal option",
        "unknown option",
        "unknown argument",
        "invalid option",
        "unrecognized option",
        " is unknown, try ",
    ];
    if option_error_markers
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return RejectionClassification {
            reason: "option-error-output".to_string(),
            failure_code: FailureCode::NotHelpOutput,
        };
    }

    let not_found_markers = [
        "command not found",
        "not recognized as an internal or external command",
        "unknown binary",
    ];
    if not_found_markers
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return RejectionClassification {
            reason: "not-installed-output".to_string(),
            failure_code: FailureCode::NotInstalled,
        };
    }

    RejectionClassification {
        reason: "not-help-output".to_string(),
        failure_code: FailureCode::NotHelpOutput,
    }
}

fn schema_has_entities(schema: &command_schema_core::CommandSchema) -> bool {
    !schema.global_flags.is_empty()
        || !schema.subcommands.is_empty()
        || !schema.positional.is_empty()
}

fn is_candidate_diagnostic_warning(message: &str) -> bool {
    message.starts_with("Medium-confidence findings kept in diagnostics:")
        || message.starts_with("Discarded low-confidence findings:")
        || message.starts_with("False-positive filters matched")
}

fn extend_unique_warnings(target: &mut Vec<String>, warnings: impl IntoIterator<Item = String>) {
    for warning in warnings {
        if !target.contains(&warning) {
            target.push(warning);
        }
    }
}

#[derive(Debug)]
struct QualityAssessment {
    accepted: bool,
    tier: QualityTier,
    reasons: Vec<String>,
}

fn assess_extraction_quality(
    run: &ExtractionRun,
    policy: ExtractionQualityPolicy,
) -> QualityAssessment {
    if !run.result.success {
        let mut reasons = Vec::new();
        if !run.report.validation_errors.is_empty() {
            reasons.push("Schema validation failed".to_string());
        } else if run
            .report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Could not get help output for"))
        {
            reasons.push("No parseable help output from probe attempts".to_string());
        } else if let Some(reason) = summarize_probe_failure_reason(&run.report.probe_attempts) {
            reasons.push(reason);
        } else {
            reasons.push("Parsing pipeline did not produce a valid schema".to_string());
        }

        return QualityAssessment {
            accepted: false,
            tier: QualityTier::Failed,
            reasons,
        };
    }

    let mut reasons = Vec::new();
    if run.report.confidence < policy.min_confidence {
        reasons.push(format!(
            "confidence {:.2} below minimum {:.2}",
            run.report.confidence, policy.min_confidence
        ));
    }
    if run.report.coverage < policy.min_coverage {
        reasons.push(format!(
            "coverage {:.2} below minimum {:.2}",
            run.report.coverage, policy.min_coverage
        ));
    }

    let accepted = reasons.is_empty() || policy.allow_low_quality;
    let tier = if reasons.is_empty() {
        if run.report.confidence >= 0.85 && run.report.coverage >= 0.6 {
            QualityTier::High
        } else {
            QualityTier::Medium
        }
    } else {
        QualityTier::Low
    };

    if !reasons.is_empty() && policy.allow_low_quality {
        reasons.push("accepted by --allow-low-quality override".to_string());
    }

    QualityAssessment {
        accepted,
        tier,
        reasons,
    }
}

fn summarize_probe_failure_reason(probe_attempts: &[ProbeAttemptReport]) -> Option<String> {
    if probe_attempts
        .iter()
        .any(|attempt| attempt.rejection_reason.as_deref() == Some("environment-blocked"))
    {
        return Some("Help probing was blocked by environment restrictions".to_string());
    }
    if probe_attempts
        .iter()
        .any(|attempt| attempt.rejection_reason.as_deref() == Some("option-error-output"))
    {
        return Some("Probe output looked like option-error output rather than help".to_string());
    }
    None
}

pub fn apply_quality_policy(
    mut run: ExtractionRun,
    policy: ExtractionQualityPolicy,
) -> ExtractionRun {
    let assessment = assess_extraction_quality(&run, policy);
    run.report.accepted_for_suggestions = assessment.accepted;
    run.report.quality_tier = assessment.tier;
    run.report.quality_reasons = assessment.reasons.clone();

    if !assessment.accepted {
        if run.result.success && !assessment.reasons.is_empty() {
            let reason = assessment.reasons.join("; ");
            let gate_warning = format!("Quality gate rejected schema: {reason}");
            extend_unique_warnings(
                &mut run.result.warnings,
                std::iter::once(gate_warning.clone()),
            );
            extend_unique_warnings(&mut run.report.warnings, std::iter::once(gate_warning));
            run.report.failure_code = Some(FailureCode::QualityRejected);
            run.report.failure_detail = Some(reason);
        }
        run.result.success = false;
        run.result.schema = None;
        run.report.success = false;
    } else {
        // Clear any failure codes on success
        run.report.failure_code = None;
        run.report.failure_detail = None;
    }

    run
}

/// Derives a structured failure code from probe attempts when no help output was found.
fn derive_probe_failure(
    probe_attempts: &[ProbeAttemptReport],
    _failure_warning: &str,
) -> (FailureCode, String) {
    // Check if all attempts indicated not-found (spawn_not_found is encoded as
    // the error message pattern or missing exit code + no output).
    let all_not_found = !probe_attempts.is_empty()
        && probe_attempts.iter().all(|a| {
            a.error
                .as_deref()
                .is_some_and(|e| e.contains("shell spawn failed") || e.contains("spawn failed"))
                || (a.exit_code.is_none() && a.output_len == 0 && a.error.is_none() && !a.timed_out)
        });
    if all_not_found {
        return (
            FailureCode::NotInstalled,
            "Command not found on the system".to_string(),
        );
    }

    if probe_attempts.iter().any(|a| a.timed_out) {
        return (
            FailureCode::Timeout,
            "All probe attempts timed out".to_string(),
        );
    }

    if probe_attempts.iter().any(|a| {
        a.rejection_reason
            .as_deref()
            .is_some_and(|r| r == "environment-blocked")
    }) {
        return (
            FailureCode::PermissionBlocked,
            "Help probing was blocked by environment restrictions".to_string(),
        );
    }

    let not_installed_hits = probe_attempts
        .iter()
        .filter(|a| {
            a.rejection_reason
                .as_deref()
                .is_some_and(|r| r == "not-installed-output")
        })
        .count();
    if not_installed_hits >= 2 {
        return (
            FailureCode::NotInstalled,
            "Command appears to be unavailable on the system".to_string(),
        );
    }

    (
        FailureCode::NotHelpOutput,
        "Probe output did not contain recognizable help text".to_string(),
    )
}

/// Extracts a complete command schema including subcommands.
pub fn extract_command_schema(command: &str) -> ExtractionResult {
    extract_command_schema_with_report(command).result
}

/// Extracts a complete command schema including subcommands and a diagnostics report.
pub fn extract_command_schema_with_report(command: &str) -> ExtractionRun {
    extract_command_schema_with_report_and_policy(command, ExtractionQualityPolicy::permissive())
}

/// Extracts a complete command schema with report and quality policy gating.
pub fn extract_command_schema_with_report_and_policy(
    command: &str,
    policy: ExtractionQualityPolicy,
) -> ExtractionRun {
    let run = extract_command_schema_with_report_base(command);
    apply_quality_policy(run, policy)
}

fn extract_command_schema_with_report_base(command: &str) -> ExtractionRun {
    let identity = resolve_command_identity(command);
    let mut warnings = Vec::new();
    let probe_run = probe_command_help_with_metadata(command);
    let probe_attempts = to_probe_attempt_reports(&probe_run.attempts);

    // Get help output
    let raw_output = match probe_run.help_output {
        Some(output) => output,
        None => {
            let failure_warning = format!("Could not get help output for '{}'", command);
            let (failure_code, failure_detail) =
                derive_probe_failure(&probe_attempts, &failure_warning);
            return ExtractionRun {
                result: ExtractionResult {
                    schema: None,
                    raw_output: String::new(),
                    detected_format: None,
                    warnings: vec![failure_warning.clone()],
                    success: false,
                },
                report: ExtractionReport {
                    command: command.to_string(),
                    resolved_executable_path: identity.resolved_executable_path.clone(),
                    resolved_implementation: identity.resolved_implementation.clone(),
                    success: false,
                    accepted_for_suggestions: false,
                    quality_tier: QualityTier::Failed,
                    quality_reasons: Vec::new(),
                    failure_code: Some(failure_code),
                    failure_detail: Some(failure_detail),
                    selected_format: None,
                    format_scores: Vec::new(),
                    parsers_used: vec!["probe-failed".to_string()],
                    confidence: 0.0,
                    coverage: 0.0,
                    relevant_lines: 0,
                    recognized_lines: 0,
                    unresolved_lines: Vec::new(),
                    probe_attempts,
                    warnings: vec![failure_warning],
                    validation_errors: Vec::new(),
                },
            };
        }
    };

    // Extract version from help output if not already set
    let detected_version = crate::version::extract_version(&raw_output, command);

    // Parse the help output
    let mut parser = HelpParser::new(command, &raw_output);
    let mut schema = match parser.parse() {
        Some(s) => s,
        None => {
            let diagnostics = parser.diagnostics().clone();
            let parser_warnings = parser.warnings().to_vec();
            let coverage = diagnostics.coverage();
            return ExtractionRun {
                result: ExtractionResult {
                    schema: None,
                    raw_output,
                    detected_format: parser.detected_format(),
                    warnings: parser_warnings.clone(),
                    success: false,
                },
                report: ExtractionReport {
                    command: command.to_string(),
                    resolved_executable_path: identity.resolved_executable_path.clone(),
                    resolved_implementation: identity.resolved_implementation.clone(),
                    success: false,
                    accepted_for_suggestions: false,
                    quality_tier: QualityTier::Failed,
                    quality_reasons: Vec::new(),
                    failure_code: Some(FailureCode::ParseFailed),
                    failure_detail: Some(
                        "Help text was found but parsing produced no schema".to_string(),
                    ),
                    selected_format: parser.detected_format().map(help_format_label),
                    format_scores: to_format_score_reports(&diagnostics.format_scores),
                    parsers_used: diagnostics.parsers_used,
                    confidence: 0.0,
                    coverage,
                    relevant_lines: diagnostics.relevant_lines,
                    recognized_lines: diagnostics.recognized_lines,
                    unresolved_lines: diagnostics.unresolved_lines,
                    probe_attempts,
                    warnings: parser_warnings,
                    validation_errors: Vec::new(),
                },
            };
        }
    };

    // Set version if detected and not already present in schema
    if schema.version.is_none() {
        schema.version = detected_version;
    }

    let parser_warnings = parser.warnings().to_vec();
    let mut report_warnings = parser_warnings.clone();
    let actionable_parser_warnings = parser_warnings
        .into_iter()
        .filter(|warning| !is_candidate_diagnostic_warning(warning));
    extend_unique_warnings(&mut warnings, actionable_parser_warnings);

    // Recursively probe subcommands
    let mut probed_subcommands = HashSet::new();
    probe_subcommands_recursive(
        command,
        &mut schema.subcommands,
        &mut probed_subcommands,
        1,
        &mut warnings,
    );

    info!(
        command = command,
        subcommands = schema.subcommands.len(),
        flags = schema.global_flags.len(),
        confidence = schema.confidence,
        "Extracted command schema"
    );

    let diagnostics = parser.diagnostics().clone();
    let validation_errors = validate_schema(&schema)
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    let has_entities = schema_has_entities(&schema);
    if !has_entities {
        warnings.push(format!(
            "Extracted schema for '{}' contains no flags, subcommands, or positional arguments",
            command
        ));
    }
    let success = validation_errors.is_empty() && has_entities;
    extend_unique_warnings(&mut report_warnings, warnings.iter().cloned());

    let (fc, fd) = if success {
        (None, None)
    } else if !validation_errors.is_empty() {
        (
            Some(FailureCode::ParseFailed),
            Some(format!(
                "Schema validation failed: {}",
                validation_errors.join("; ")
            )),
        )
    } else {
        (
            Some(FailureCode::ParseFailed),
            Some("Parsed schema contains no entities".to_string()),
        )
    };

    let report = ExtractionReport {
        command: command.to_string(),
        resolved_executable_path: identity.resolved_executable_path,
        resolved_implementation: identity.resolved_implementation,
        success,
        accepted_for_suggestions: false,
        quality_tier: QualityTier::Failed,
        quality_reasons: Vec::new(),
        failure_code: fc,
        failure_detail: fd,
        selected_format: parser.detected_format().map(help_format_label),
        format_scores: to_format_score_reports(&diagnostics.format_scores),
        parsers_used: diagnostics.parsers_used.clone(),
        confidence: schema.confidence,
        coverage: diagnostics.coverage(),
        relevant_lines: diagnostics.relevant_lines,
        recognized_lines: diagnostics.recognized_lines,
        unresolved_lines: diagnostics.unresolved_lines,
        probe_attempts,
        warnings: report_warnings,
        validation_errors: validation_errors.clone(),
    };

    ExtractionRun {
        result: ExtractionResult {
            schema: if success { Some(schema) } else { None },
            raw_output,
            detected_format: parser.detected_format(),
            warnings,
            success,
        },
        report,
    }
}

/// Recursively probes subcommands to get their schemas.
fn probe_subcommands_recursive(
    base_command: &str,
    subcommands: &mut [SubcommandSchema],
    probed: &mut HashSet<String>,
    depth: usize,
    warnings: &mut Vec<String>,
) {
    let sibling_names = subcommands
        .iter()
        .map(|subcmd| subcmd.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();

    for subcmd in subcommands.iter_mut() {
        let full_command = format!("{} {}", base_command, subcmd.name);

        // Skip if already probed (avoid cycles)
        if probed.contains(&full_command) {
            continue;
        }
        if probed.len() >= MAX_RECURSIVE_PROBE_BUDGET {
            extend_unique_warnings(
                warnings,
                std::iter::once(format!(
                    "Reached recursive probe budget ({MAX_RECURSIVE_PROBE_BUDGET}) while probing '{}'; skipping deeper discovery",
                    base_command
                )),
            );
            break;
        }
        probed.insert(full_command.clone());

        // Skip common non-probeable subcommands
        if should_skip_subcommand(&subcmd.name) {
            continue;
        }
        if should_skip_known_cycle_prone_probe(base_command, subcmd) {
            continue;
        }

        debug!(
            command = %full_command,
            depth = depth,
            "Probing subcommand"
        );

        // Get help for this subcommand
        if let Some(help_output) = probe_command_help(&full_command) {
            let mut parser = HelpParser::new(&full_command, &help_output);
            if let Some(sub_schema) = parser.parse() {
                // Some CLIs (notably apt-family) print parent-level help for
                // "<command> <subcommand> --help". When that happens, merging
                // parsed output would incorrectly inject sibling command list
                // data (and generic positionals) into every subcommand.
                if is_parent_help_echo_for_subcommand(&subcmd.name, &sub_schema, &sibling_names) {
                    continue;
                }

                // Merge extracted info into subcommand
                subcmd.flags = sub_schema.global_flags;
                subcmd.positional = sub_schema.positional;
                // Preserve the parent help's subcommand description when present;
                // recursive subcommand probes often start with generic banners.
                subcmd.description = subcmd.description.take().or(sub_schema.description);

                // Add nested subcommands
                let mut nested_subcommands = sub_schema.subcommands;
                if nested_subcommands
                    .iter()
                    .any(|nested| nested.name == subcmd.name)
                {
                    warnings.push(format!(
                        "Skipping nested subcommands for '{}' due to detected self-cycle",
                        full_command
                    ));
                    nested_subcommands.clear();
                }
                subcmd.subcommands = nested_subcommands;

                // Recurse into nested subcommands
                if !subcmd.subcommands.is_empty() {
                    probe_subcommands_recursive(
                        &full_command,
                        &mut subcmd.subcommands,
                        probed,
                        depth + 1,
                        warnings,
                    );
                }
            }
            let actionable = parser
                .warnings()
                .iter()
                .filter(|message| !is_candidate_diagnostic_warning(message))
                .cloned()
                .collect::<Vec<_>>();
            extend_unique_warnings(warnings, actionable);
        }
    }
}

fn is_parent_help_echo_for_subcommand(
    subcommand_name: &str,
    parsed_sub_schema: &command_schema_core::CommandSchema,
    sibling_names: &HashSet<String>,
) -> bool {
    if parsed_sub_schema.subcommands.len() < 2 {
        return false;
    }

    let parsed_names = parsed_sub_schema
        .subcommands
        .iter()
        .map(|sub| sub.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();

    // The echoed parent help usually includes the currently probed subcommand
    // plus many of its siblings.
    if !parsed_names.contains(&subcommand_name.to_ascii_lowercase()) {
        return false;
    }

    let sibling_overlap = parsed_names.intersection(sibling_names).count();
    sibling_overlap >= 3
}

/// Determines if a subcommand should be skipped during probing.
fn should_skip_subcommand(name: &str) -> bool {
    // Skip help-related subcommands (they don't have meaningful help of their own)
    let skip_list = ["help", "version", "completion", "completions"];
    skip_list.contains(&name)
}

fn should_skip_known_cycle_prone_probe(base_command: &str, subcmd: &SubcommandSchema) -> bool {
    let base = base_command
        .split_whitespace()
        .next()
        .unwrap_or(base_command)
        .to_ascii_lowercase();
    let desc = subcmd
        .description
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();

    if base == "stty" {
        return desc.starts_with("same as ")
            || desc.starts_with("print ")
            || desc.starts_with("set ");
    }

    if base == "tar" {
        let is_format_label = matches!(
            subcmd.name.as_str(),
            "gnu" | "oldgnu" | "pax" | "posix" | "ustar" | "v7"
        );
        return is_format_label || desc.starts_with("same as ");
    }

    false
}

/// Extracts schemas for multiple commands.
pub fn extract_multiple_schemas(commands: &[&str]) -> Vec<ExtractionResult> {
    commands
        .iter()
        .map(|cmd| extract_command_schema(cmd))
        .collect()
}

pub fn to_format_score_reports(scores: &[FormatScore]) -> Vec<FormatScoreReport> {
    scores
        .iter()
        .map(|entry| FormatScoreReport {
            format: help_format_label(entry.format),
            score: entry.score,
        })
        .collect()
}

fn to_probe_attempt_reports(attempts: &[ProbeAttempt]) -> Vec<ProbeAttemptReport> {
    attempts
        .iter()
        .map(|attempt| ProbeAttemptReport {
            help_flag: attempt.help_flag.clone(),
            argv: attempt.argv.clone(),
            exit_code: attempt.exit_code,
            timed_out: attempt.timed_out,
            error: attempt.error.clone(),
            rejection_reason: attempt.rejection_reason.clone(),
            output_source: attempt.output_source.clone(),
            output_len: attempt.output_len,
            output_preview: attempt.output_preview.clone(),
            accepted: attempt.accepted,
        })
        .collect()
}

pub fn help_format_label(format: HelpFormat) -> String {
    match format {
        HelpFormat::Clap => "clap",
        HelpFormat::Cobra => "cobra",
        HelpFormat::Argparse => "argparse",
        HelpFormat::Docopt => "docopt",
        HelpFormat::Gnu => "gnu",
        HelpFormat::Bsd => "bsd",
        HelpFormat::Man => "man",
        HelpFormat::Unknown => "unknown",
    }
    .to_string()
}

/// Probes a command to check if it exists and has help.
pub fn command_exists(command: &str) -> bool {
    Command::new("which")
        .arg(command)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn resolve_command_identity(command: &str) -> CommandIdentity {
    let base = command.split_whitespace().next().unwrap_or(command);
    if base.is_empty() {
        return CommandIdentity::default();
    }

    let output = match Command::new("which").arg(base).output() {
        Ok(output) if output.status.success() => output,
        _ => return CommandIdentity::default(),
    };

    let raw = String::from_utf8_lossy(&output.stdout);
    let first = raw.lines().next().unwrap_or_default().trim();
    if first.is_empty() {
        return CommandIdentity::default();
    }

    let canonical = fs::canonicalize(first).ok().unwrap_or_else(|| first.into());
    let resolved_impl = Path::new(&canonical)
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned);
    // Store only the basename to avoid leaking absolute filesystem paths.
    let resolved_basename = resolved_impl.clone();

    CommandIdentity {
        resolved_executable_path: resolved_basename,
        resolved_implementation: resolved_impl,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use command_schema_core::{
        CommandSchema, ExtractionResult, HelpFormat, SchemaSource, SubcommandSchema,
    };

    #[test]
    fn test_is_help_output() {
        assert!(is_help_output(
            "Usage: mycommand [options]\n\nOptions:\n  --help"
        ));
        assert!(is_help_output(
            "USAGE:\n    myapp [FLAGS]\n\nFLAGS:\n    -h, --help"
        ));
        assert!(is_help_output(
            "umask: umask [-p] [-S] [mode]\n\nOptions:\n  -p output may be reused as input\n\nExit Status:\nReturns success unless MODE is invalid or an invalid option is given."
        ));
        assert!(!is_help_output("error: command not found"));
        assert!(!is_help_output("short"));
        assert!(!is_help_output("sh: 0: Illegal option -h"));
        assert!(!is_help_output("bzexe: --help not a file"));
        assert!(!is_help_output(
            "Argument \"--help\" is unknown, try \"rtmon help\"."
        ));
        assert!(is_help_output(
            "Illegal option --\nUsage: /usr/bin/which [-as] args"
        ));
    }

    #[test]
    fn test_classify_rejection_detects_not_installed_and_display_blocked() {
        let not_found = classify_rejection("bash: line 1: npx: command not found");
        assert_eq!(not_found.failure_code, FailureCode::NotInstalled);
        assert_eq!(not_found.reason, "not-installed-output");

        let display = classify_rejection("Error: Can't open display:");
        assert_eq!(display.failure_code, FailureCode::PermissionBlocked);
        assert_eq!(display.reason, "environment-blocked");
    }

    #[test]
    fn test_derive_probe_failure_prefers_not_installed_when_repeated() {
        let attempts = vec![
            ProbeAttemptReport {
                help_flag: "--help".to_string(),
                argv: vec!["npx".to_string(), "--help".to_string()],
                exit_code: Some(127),
                timed_out: false,
                error: None,
                rejection_reason: Some("not-installed-output".to_string()),
                output_source: Some("stderr".to_string()),
                output_len: 32,
                output_preview: Some("bash: npx: command not found".to_string()),
                accepted: false,
            },
            ProbeAttemptReport {
                help_flag: "-h".to_string(),
                argv: vec!["npx".to_string(), "-h".to_string()],
                exit_code: Some(127),
                timed_out: false,
                error: None,
                rejection_reason: Some("not-installed-output".to_string()),
                output_source: Some("stderr".to_string()),
                output_len: 32,
                output_preview: Some("bash: npx: command not found".to_string()),
                accepted: false,
            },
        ];

        let (code, detail) = derive_probe_failure(&attempts, "missing");
        assert_eq!(code, FailureCode::NotInstalled);
        assert!(detail.contains("unavailable"));
    }

    #[test]
    fn test_command_specific_probe_suffixes_for_ps() {
        let suffixes = command_specific_probe_suffixes("ps");
        assert!(
            suffixes
                .iter()
                .any(|suffix| suffix == &vec!["--help", "all"])
        );
    }

    #[test]
    fn test_man_probe_pages_prefers_specific_chain() {
        let pages = man_probe_pages(&["git", "remote", "add"]);
        assert_eq!(pages, vec!["git-remote-add", "git-remote"]);

        let single = man_probe_pages(&["stat"]);
        assert_eq!(single, vec!["stat"]);
    }

    #[test]
    fn test_should_probe_help_subcommand_from_hint_for_single_command() {
        let attempts = vec![ProbeAttempt {
            help_flag: "--help".to_string(),
            argv: vec!["rtmon".to_string(), "--help".to_string()],
            exit_code: Some(255),
            timed_out: false,
            error: None,
            rejection_reason: Some("option-error-output".to_string()),
            failure_code: Some(FailureCode::NotHelpOutput),
            output_source: Some("stderr".to_string()),
            output_len: 48,
            output_preview: Some("Argument \"--help\" is unknown, try \"rtmon help\".".to_string()),
            accepted: false,
        }];

        assert!(should_probe_help_subcommand(&["rtmon"], &attempts));
    }

    #[test]
    fn test_should_not_probe_help_subcommand_without_hint_for_single_command() {
        let attempts = vec![ProbeAttempt {
            help_flag: "--help".to_string(),
            argv: vec!["bzexe".to_string(), "--help".to_string()],
            exit_code: Some(1),
            timed_out: false,
            error: None,
            rejection_reason: Some("not-help-output".to_string()),
            failure_code: Some(FailureCode::NotHelpOutput),
            output_source: Some("stdout".to_string()),
            output_len: 25,
            output_preview: Some("bzexe: --help not a file".to_string()),
            accepted: false,
        }];

        assert!(!should_probe_help_subcommand(&["bzexe"], &attempts));
    }

    #[test]
    fn test_command_specific_probe_env_for_ansible() {
        let env = command_specific_probe_env("ansible");
        assert!(
            env.iter()
                .any(|(key, value)| *key == "ANSIBLE_LOCAL_TEMP" && *value == "/tmp")
        );
    }

    #[test]
    fn test_adapt_service_help_output_adds_structured_sections() {
        let raw = "Usage: service < option > | --status-all | [ service_name [ command | --full-restart ] ]\n";
        let adapted = adapt_service_help_output(raw);
        assert!(adapted.contains("Options:"));
        assert!(adapted.contains("--status-all"));
        assert!(adapted.contains("Arguments:"));
    }

    #[test]
    fn test_schema_has_entities() {
        let empty = CommandSchema::new("svc", SchemaSource::HelpCommand);
        assert!(!schema_has_entities(&empty));

        let mut with_flag = CommandSchema::new("svc", SchemaSource::HelpCommand);
        with_flag
            .global_flags
            .push(command_schema_core::FlagSchema::boolean(
                Some("-v"),
                Some("--verbose"),
            ));
        assert!(schema_has_entities(&with_flag));
    }

    #[test]
    fn test_quality_policy_rejects_low_quality_success() {
        let run = ExtractionRun {
            result: ExtractionResult {
                schema: Some(CommandSchema::new("svc", SchemaSource::HelpCommand)),
                raw_output: "Usage: svc".to_string(),
                detected_format: Some(HelpFormat::Unknown),
                warnings: Vec::new(),
                success: true,
            },
            report: ExtractionReport {
                command: "svc".to_string(),
                resolved_executable_path: None,
                resolved_implementation: None,
                success: true,
                accepted_for_suggestions: false,
                quality_tier: QualityTier::Failed,
                quality_reasons: Vec::new(),
                failure_code: None,
                failure_detail: None,
                selected_format: Some("unknown".to_string()),
                format_scores: Vec::new(),
                parsers_used: vec!["test".to_string()],
                confidence: 0.4,
                coverage: 0.1,
                relevant_lines: 10,
                recognized_lines: 1,
                unresolved_lines: Vec::new(),
                probe_attempts: Vec::new(),
                warnings: Vec::new(),
                validation_errors: Vec::new(),
            },
        };

        let gated = apply_quality_policy(
            run,
            ExtractionQualityPolicy {
                min_confidence: 0.7,
                min_coverage: 0.2,
                allow_low_quality: false,
            },
        );

        assert!(!gated.result.success);
        assert!(gated.result.schema.is_none());
        assert!(!gated.report.accepted_for_suggestions);
        assert_eq!(gated.report.quality_tier, QualityTier::Low);
        assert!(
            gated
                .report
                .quality_reasons
                .iter()
                .any(|reason| reason.contains("confidence"))
        );
    }

    #[test]
    fn test_should_skip_subcommand() {
        assert!(should_skip_subcommand("help"));
        assert!(should_skip_subcommand("version"));
        assert!(should_skip_subcommand("completion"));
        assert!(!should_skip_subcommand("build"));
        assert!(!should_skip_subcommand("run"));
    }

    #[test]
    fn test_should_skip_known_cycle_prone_probe_for_stty_settings() {
        let mut setting = SubcommandSchema::new("speed");
        setting.description = Some("print the terminal speed".to_string());
        assert!(should_skip_known_cycle_prone_probe("stty", &setting));
    }

    #[test]
    fn test_should_skip_known_cycle_prone_probe_for_tar_format_labels() {
        let mut format = SubcommandSchema::new("posix");
        format.description = Some("same as pax".to_string());
        assert!(should_skip_known_cycle_prone_probe("tar", &format));
    }

    #[test]
    fn test_should_not_skip_regular_subcommand_probe() {
        let mut sub = SubcommandSchema::new("install");
        sub.description = Some("Install packages".to_string());
        assert!(!should_skip_known_cycle_prone_probe("apt-get", &sub));
    }

    #[test]
    fn test_extract_report_contains_probe_attempt_metadata_on_probe_failure() {
        let run = extract_command_schema_with_report("__wrashpty_missing_command__");
        assert!(!run.result.success);
        assert!(run.report.probe_attempts.len() >= HELP_FLAGS.len());
        assert!(
            run.report
                .probe_attempts
                .first()
                .is_some_and(|attempt| attempt.help_flag.starts_with("man:"))
        );

        for attempt in &run.report.probe_attempts {
            assert!(attempt.error.is_some() || attempt.timed_out || attempt.exit_code.is_some());
        }
    }

    #[test]
    fn test_detect_parent_help_echo_for_subcommand() {
        let mut parsed = CommandSchema::new("apt install", SchemaSource::HelpCommand);
        parsed.subcommands = vec![
            SubcommandSchema::new("install"),
            SubcommandSchema::new("remove"),
            SubcommandSchema::new("update"),
            SubcommandSchema::new("upgrade"),
        ];

        let sibling_names = ["install", "remove", "update", "upgrade"]
            .into_iter()
            .map(str::to_string)
            .collect::<std::collections::HashSet<_>>();

        assert!(is_parent_help_echo_for_subcommand(
            "install",
            &parsed,
            &sibling_names
        ));
    }

    #[test]
    fn test_non_echo_subcommand_schema_is_not_treated_as_parent_echo() {
        let mut parsed = CommandSchema::new("git remote", SchemaSource::HelpCommand);
        parsed.subcommands = vec![
            SubcommandSchema::new("add"),
            SubcommandSchema::new("remove"),
        ];

        let sibling_names = ["remote", "commit", "push"]
            .into_iter()
            .map(str::to_string)
            .collect::<std::collections::HashSet<_>>();

        assert!(!is_parent_help_echo_for_subcommand(
            "remote",
            &parsed,
            &sibling_names
        ));
    }

    // Integration tests - only run if commands are available
    #[test]
    #[ignore] // Run with: cargo test -- --ignored
    fn test_extract_git_schema() {
        if !command_exists("git") {
            return;
        }

        let result = extract_command_schema("git");
        assert!(result.success);

        let schema = result.schema.unwrap();
        assert!(!schema.subcommands.is_empty());

        // Should have common git subcommands
        assert!(schema.find_subcommand("commit").is_some());
        assert!(schema.find_subcommand("push").is_some());
    }

    #[test]
    #[ignore]
    fn test_extract_cargo_schema() {
        if !command_exists("cargo") {
            return;
        }

        let result = extract_command_schema("cargo");
        assert!(result.success);

        let schema = result.schema.unwrap();
        assert!(schema.find_subcommand("build").is_some());
        assert!(schema.find_subcommand("test").is_some());
    }

    #[test]
    #[ignore]
    fn test_probe_git_commit_help() {
        if !command_exists("git") {
            return;
        }

        let help = probe_command_help("git commit");
        assert!(help.is_some());

        let help_text = help.unwrap();
        assert!(help_text.contains("-m") || help_text.contains("--message"));
    }
}
