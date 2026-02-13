//! Format classification with weighted scoring.
//!
//! Detects help-output formats and applies hard-negative filtering helpers
//! used to suppress false positives.

use command_schema_core::HelpFormat;

use super::strategies::man::rendered::sections::normalize_section_name;
use super::util::looks_like_man_title_line;
use super::{FormatScore, IndexedLine};

/// Scores the given help output lines against known `HelpFormat` variants.
/// Returns a descending-sorted vector of `FormatScore` entries.
pub fn classify_formats(lines: &[&str]) -> Vec<FormatScore> {
    let mut scores = vec![
        FormatScore {
            format: HelpFormat::Clap,
            score: 0.0,
        },
        FormatScore {
            format: HelpFormat::Cobra,
            score: 0.0,
        },
        FormatScore {
            format: HelpFormat::Gnu,
            score: 0.0,
        },
        FormatScore {
            format: HelpFormat::Argparse,
            score: 0.0,
        },
        FormatScore {
            format: HelpFormat::Docopt,
            score: 0.0,
        },
        FormatScore {
            format: HelpFormat::Bsd,
            score: 0.0,
        },
        FormatScore {
            format: HelpFormat::Man,
            score: 0.0,
        },
        FormatScore {
            format: HelpFormat::Unknown,
            score: 0.05,
        },
    ];

    let output = lines.join("\n");
    for score in &mut scores {
        score.score += match score.format {
            HelpFormat::Clap => {
                let mut s = 0.0;
                if output.contains("USAGE:") {
                    s += 0.35;
                }
                if output.contains("FLAGS:") {
                    s += 0.25;
                }
                if output.contains("OPTIONS:") {
                    s += 0.2;
                }
                if output.contains("SUBCOMMANDS:") || output.contains("Commands:") {
                    s += 0.2;
                }
                s
            }
            HelpFormat::Cobra => {
                let mut s = 0.0;
                if output.contains("Available Commands:") {
                    s += 0.5;
                }
                if output.contains("Use \"") && output.contains("--help") {
                    s += 0.35;
                }
                if output.contains("Flags:") {
                    s += 0.15;
                }
                s
            }
            HelpFormat::Gnu => {
                let mut s = 0.0;
                if output.contains("Usage:") {
                    s += 0.25;
                }
                if output.contains("--help") {
                    s += 0.2;
                }
                if output.contains("--version") {
                    s += 0.2;
                }
                if lines.iter().any(|line| line.trim_start().starts_with('-')) {
                    s += 0.2;
                }
                s
            }
            HelpFormat::Argparse => {
                let mut s = 0.0;
                if output.contains("positional arguments:") {
                    s += 0.45;
                }
                if output.contains("optional arguments:") {
                    s += 0.45;
                }
                s
            }
            HelpFormat::Docopt => {
                if output.starts_with("Usage:") {
                    0.75
                } else {
                    0.0
                }
            }
            HelpFormat::Bsd => {
                if output.contains("SYNOPSIS") || output.contains("DESCRIPTION") {
                    0.45
                } else {
                    0.0
                }
            }
            HelpFormat::Man => score_man_format(lines),
            HelpFormat::Unknown => 0.0,
        };
    }

    scores.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scores
}

fn score_man_format(lines: &[&str]) -> f64 {
    let raw_macro_count = lines
        .iter()
        .take(20)
        .filter(|line| is_roff_macro_line(line))
        .count();

    let lower = lines
        .iter()
        .map(|line| line.to_ascii_lowercase())
        .collect::<Vec<_>>();

    let has_mdoc_markers = lower.iter().any(|line| {
        line.starts_with(".dt ") || line.starts_with(".dd ") || line.starts_with(".sh ")
    });
    let has_man_markers = lower.iter().any(|line| {
        line.starts_with(".th ") || line.starts_with(".sh ") || line.starts_with(".tp")
    });

    let mut score: f64 = 0.0;
    if raw_macro_count >= 3 {
        score = 0.95;
    } else if raw_macro_count >= 2 {
        score = 0.90;
    }

    if score > 0.0 {
        if has_mdoc_markers {
            score += 0.05;
        }
        if has_man_markers {
            score += 0.05;
        }
        return score.clamp(0.0, 1.0);
    }

    let rendered_header_hits = lines
        .iter()
        .take(12)
        .filter(|line| {
            let trimmed = line.trim();
            looks_like_man_title_line(trimmed)
        })
        .count();

    let section_hits = lines
        .iter()
        .filter(|line| looks_like_rendered_man_section_header(line))
        .count();

    if rendered_header_hits > 0 {
        score += 0.80;
    }
    score += (section_hits.min(4) as f64) * 0.10;
    score.clamp(0.0, 1.0)
}

fn is_roff_macro_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    if first != '.' && first != '\'' {
        return false;
    }
    trimmed.chars().nth(1).is_some_and(|ch| ch.is_ascii_alphabetic())
}

fn looks_like_rendered_man_section_header(line: &str) -> bool {
    normalize_section_name(line.trim()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_man_raw_roff_prefers_man() {
        let lines = [".TH GIT-REBASE 1", ".SH NAME", ".TP", ".B --continue"];
        let scores = classify_formats(&lines);
        assert_eq!(
            scores.first().map(|score| score.format),
            Some(HelpFormat::Man)
        );
    }

    #[test]
    fn test_classify_man_rendered_prefers_man() {
        let lines = [
            "GIT-REBASE(1)                     Git Manual                     GIT-REBASE(1)",
            "NAME",
            "SYNOPSIS",
            "OPTIONS",
        ];
        let scores = classify_formats(&lines);
        assert_eq!(
            scores.first().map(|score| score.format),
            Some(HelpFormat::Man)
        );
    }

    #[test]
    fn test_classify_man_rendered_prefers_man_with_uppercase_section_codes() {
        let lines = [
            "FOO(1M)                     User Commands                     FOO(1M)",
            "BAR(3P)",
            "NAME",
            "SYNOPSIS",
            "DESCRIPTION",
        ];
        let scores = classify_formats(&lines);
        assert_eq!(
            scores.first().map(|score| score.format),
            Some(HelpFormat::Man)
        );
    }
}

/// Returns `true` if `text` matches a common placeholder token (e.g. COMMAND, FILE, ARG).
pub fn is_placeholder_token(text: &str) -> bool {
    matches!(
        text.trim().to_ascii_uppercase().as_str(),
        "COMMAND"
            | "FILE"
            | "PATH"
            | "URL"
            | "ARG"
            | "OPTION"
            | "SUBCOMMAND"
            | "CMD"
            | "ARGS"
            | "OPTIONS"
    )
}

/// Returns `true` if `line` looks like an environment variable assignment row
/// (e.g. `export FOO=bar` or `MY_VAR=value`).
pub fn is_env_var_row(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.starts_with("export ") {
        return true;
    }

    let Some((left, _)) = trimmed.split_once('=') else {
        return false;
    };

    let key = left.trim();
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}

/// Returns `true` if `line` contains keybinding-like patterns (Ctrl+, ^, Esc-, arrow keys).
pub fn is_keybinding_row(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.contains("Ctrl+") || trimmed.contains("ctrl+") || trimmed.contains('^') {
        return true;
    }

    let lower = trimmed.to_ascii_lowercase();
    lower.contains("esc-")
        || lower.contains("arrow")
        || lower.contains("backspace")
        || lower.contains("delete")
}

/// Returns `true` if `line` matches a table-like prose header
/// (e.g. "name  description", "command  description").
pub fn is_prose_header(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "name  description"
            | "name description"
            | "command  description"
            | "command description"
            | "option  description"
            | "option description"
    )
}

/// Counts how many of the given `IndexedLine`s match hard-negative filters
/// (env var rows, keybinding rows, or prose headers).
pub fn count_filter_hits(lines: &[IndexedLine]) -> usize {
    lines
        .iter()
        .filter(|line| {
            is_env_var_row(line.text.as_str())
                || is_keybinding_row(line.text.as_str())
                || is_prose_header(line.text.as_str())
        })
        .count()
}
