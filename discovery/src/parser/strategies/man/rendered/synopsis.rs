//! SYNOPSIS parser for rendered man pages.

use std::collections::HashSet;

use command_schema_core::{ArgSchema, FlagSchema, SubcommandSchema, ValueType};

use crate::parser::ast::{ArgCandidate, FlagCandidate, SourceSpan, SubcommandCandidate};

use super::sections::ManSection;

pub fn parse_synopsis_flags(section: &ManSection) -> Vec<FlagCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for line in &section.lines {
        let trimmed = line.text.trim();
        if trimmed.is_empty() {
            continue;
        }

        let tokens = trimmed
            .split_whitespace()
            .map(|token| {
                token.trim_matches(|ch: char| {
                    matches!(
                        ch,
                        '[' | ']' | '<' | '>' | '{' | '}' | '(' | ')' | ',' | ';'
                    )
                })
            })
            .collect::<Vec<_>>();

        let mut idx = 0usize;
        while idx < tokens.len() {
            let token = tokens[idx];
            if !token.starts_with('-') {
                idx += 1;
                continue;
            }

            let aliases = split_flag_aliases(token);
            for alias in aliases {
                let (raw_name, inline_value) = alias
                    .split_once('=')
                    .map(|(head, _)| (head, true))
                    .unwrap_or((alias.as_str(), false));

                let name = normalize_flag_name(raw_name);
                if !is_valid_flag_name(&name) {
                    continue;
                }

                let mut schema = if name.starts_with("--") {
                    FlagSchema::boolean(None, Some(&name))
                } else {
                    // Treat all single-dash forms as short-style flags to
                    // avoid invalid long names like "-foo".
                    FlagSchema::boolean(Some(&name), None)
                };

                if inline_value {
                    schema.takes_value = true;
                    schema.value_type = ValueType::String;
                } else if let Some(next) = tokens.get(idx + 1)
                    && looks_like_value_placeholder(next)
                {
                    schema.takes_value = true;
                    schema.value_type = infer_value_type(next);
                }

                let key = schema
                    .long
                    .clone()
                    .or(schema.short.clone())
                    .unwrap_or_default();
                if !key.is_empty() && seen.insert(key) {
                    out.push(FlagCandidate::from_schema(
                        schema,
                        SourceSpan::single(line.index),
                        "man-rendered-synopsis-flags",
                        0.70,
                    ));
                }
            }

            idx += 1;
        }
    }

    out
}

pub fn parse_synopsis_args(section: &ManSection) -> Vec<ArgCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    // Pre-compute subcommand names from the full joined synopsis so that
    // continuation lines (which lack the root command token) still filter
    // correctly.
    let joined = join_synopsis_text(section);
    let all_subcommands = extract_synopsis_subcommand_heads(&joined);

    // Collect all flag names so we can skip their value placeholders.
    let synopsis_flags = collect_synopsis_flag_names(section);

    // Extract the leading unbracketed command tokens from the first non-empty
    // synopsis line (e.g. "git add" → {"git", "add"}, "apt-get" → {"apt-get"})
    // so we can filter them from positional candidates.
    let command_tokens: HashSet<String> = section
        .lines
        .iter()
        .map(|l| l.text.trim())
        .find(|t| !t.is_empty())
        .map(|first| {
            first
                .split_whitespace()
                .take_while(|w| {
                    !w.starts_with('-')
                        && !w.contains('[')
                        && !w.contains('<')
                        && !w.contains('{')
                        && !w.contains('(')
                })
                .map(|w| normalize_synopsis_arg_token(w).to_ascii_lowercase())
                .filter(|w| !w.is_empty())
                .collect()
        })
        .unwrap_or_default();

    for line in &section.lines {
        let trimmed = line.text.trim();
        if trimmed.is_empty() {
            continue;
        }

        let words: Vec<&str> = trimmed.split_whitespace().collect();
        let mut idx = 0;
        while idx < words.len() {
            let raw = words[idx];

            // Track flag tokens and skip their value placeholder.
            if raw.starts_with('-') || normalize_synopsis_arg_token(raw).starts_with('-') {
                // If the next token looks like a value placeholder for this
                // flag (e.g. `--depth <n>`), skip it too.
                if idx + 1 < words.len() {
                    let next = words[idx + 1];
                    let next_norm = normalize_synopsis_arg_token(next);
                    if !next.starts_with('-')
                        && !next_norm.starts_with('-')
                        && (next.contains('<')
                            || next.contains('>')
                            || next_norm
                                .chars()
                                .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch == '-'))
                    {
                        idx += 2;
                        continue;
                    }
                }
                idx += 1;
                continue;
            }

            let bracketed = raw.contains('[') || raw.contains('<') || raw.contains('{');
            let token = normalize_synopsis_arg_token(raw);
            if token.is_empty() || token.starts_with('-') {
                idx += 1;
                continue;
            }

            let token_lower = token.to_ascii_lowercase();

            // Skip command name tokens at any position (e.g. "git", "add"
            // from "git add [options] <pathspec>...").
            if command_tokens.contains(&token_lower) && !bracketed {
                idx += 1;
                continue;
            }

            // Synopsis lines are usually "<command> [args...]"; avoid treating the
            // command token itself as a positional arg when unbracketed.
            if idx == 0 && !bracketed {
                idx += 1;
                continue;
            }

            let required = !raw.contains('[');
            let multiple = raw.contains("...");
            if !looks_like_arg_token(&token) {
                idx += 1;
                continue;
            }
            if is_placeholder_command_token(&token_lower) {
                idx += 1;
                continue;
            }
            if all_subcommands.contains(&token_lower) {
                idx += 1;
                continue;
            }
            // Skip tokens that match a known flag's value name (e.g. "depth"
            // from `--depth <depth>`).
            if synopsis_flags.contains(&token_lower) {
                idx += 1;
                continue;
            }

            let name = token_lower;
            if !seen.insert(name.clone()) {
                idx += 1;
                continue;
            }

            let mut schema = if required {
                ArgSchema::required(&name, infer_value_type(&token))
            } else {
                ArgSchema::optional(&name, infer_value_type(&token))
            };
            schema.multiple = multiple;

            out.push(ArgCandidate::from_schema(
                schema,
                SourceSpan::single(line.index),
                "man-rendered-synopsis-args",
                0.75,
            ));
            idx += 1;
        }
    }

    out
}

/// Collects flag body names from the synopsis (e.g. `--depth` → "depth",
/// `--upload-pack` → "upload-pack") so they can be filtered from positional
/// candidates.
fn collect_synopsis_flag_names(section: &ManSection) -> HashSet<String> {
    let mut names = HashSet::new();
    for line in &section.lines {
        for raw in line.text.split_whitespace() {
            let token = normalize_synopsis_arg_token(raw);
            if let Some(body) = token.strip_prefix("--") {
                let clean = body
                    .split_once('=')
                    .map_or(body, |(head, _)| head)
                    .trim_end_matches(|ch: char| matches!(ch, '[' | '<'));
                if !clean.is_empty() {
                    names.insert(clean.to_ascii_lowercase());
                }
            }
        }
    }
    names
}

pub fn parse_synopsis_subcommands(section: &ManSection) -> Vec<SubcommandCandidate> {
    // Join all synopsis lines so that pipe-separated subcommand alternatives
    // that span multiple continuation lines are recognized together.
    let joined = join_synopsis_text(section);
    let names = extract_synopsis_subcommand_heads(&joined);
    let span_index = section.lines.first().map_or(0, |l| l.index);

    names
        .into_iter()
        .map(|name| {
            let sub = SubcommandSchema::new(name.as_str());
            SubcommandCandidate::from_schema(
                sub,
                SourceSpan::single(span_index),
                "man-rendered-synopsis-subcommands",
                0.78,
            )
        })
        .collect()
}

fn looks_like_arg_token(token: &str) -> bool {
    token
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && token.chars().any(|ch| ch.is_ascii_alphanumeric())
}

fn looks_like_value_placeholder(token: &str) -> bool {
    let cleaned = normalize_synopsis_arg_token(token);
    looks_like_arg_token(&cleaned)
}

fn normalize_synopsis_arg_token(raw: &str) -> String {
    raw.trim_matches(|ch: char| {
        matches!(
            ch,
            '[' | ']' | '<' | '>' | '{' | '}' | '(' | ')' | ',' | ';' | '.'
        )
    })
    .trim_end_matches("...")
    .trim()
    .to_string()
}

fn extract_synopsis_subcommand_heads(line: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    if !line.contains('|') {
        return out;
    }

    // Strip parenthesized groups that contain pipes — these are flag-value
    // alternatives (e.g. `(amend|reword)`, `(direct|inherit)`) not subcommands.
    // Also strip `=[...]` and `=<...>` patterns for the same reason.
    let cleaned = strip_flag_value_alternatives(line);

    if !cleaned.contains('|') {
        return out;
    }

    let root = cleaned
        .split_whitespace()
        .next()
        .map(|t| normalize_synopsis_arg_token(t))
        .unwrap_or_default();
    if !looks_like_command_name(&root) {
        return out;
    }
    let root_lower = root.to_ascii_lowercase();

    for segment in cleaned.split('|') {
        // Scan past the root command name and any flag-like or non-command
        // tokens to find the first subcommand candidate in this segment.
        for raw in segment.split_whitespace() {
            let token = normalize_synopsis_arg_token(raw);
            if token.is_empty() {
                continue;
            }
            let token_lower = token.to_ascii_lowercase();
            if token_lower == root_lower
                || token.starts_with('-')
                || raw.contains('<') || raw.contains('>')
                || !looks_like_command_name(&token)
                || is_placeholder_command_token(&token_lower)
            {
                continue;
            }

            out.insert(token_lower);
            break;
        }
    }

    // Require at least 2 distinct candidates to avoid false positives from
    // synopsis lines that use pipes only for flag alternatives
    // (e.g. "git rebase [-i | --interactive] ... (--continue | --abort)").
    if out.len() < 2 {
        out.clear();
    }

    out
}

/// Removes parenthesized groups that contain `|` (flag-value alternatives like
/// `(amend|reword)` or `(direct|inherit)`) and `=`-prefixed bracketed groups
/// from the synopsis text. This prevents flag value enums from being
/// misidentified as subcommand alternatives.
fn strip_flag_value_alternatives(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Match `=(...)` or `=[...]` patterns: skip from `=` to closing bracket.
        if chars[i] == '=' && i + 1 < chars.len() && matches!(chars[i + 1], '(' | '[') {
            let close = if chars[i + 1] == '(' { ')' } else { ']' };
            let mut depth = 1;
            let mut j = i + 2;
            while j < chars.len() && depth > 0 {
                if chars[j] == chars[i + 1] {
                    depth += 1;
                } else if chars[j] == close {
                    depth -= 1;
                }
                j += 1;
            }
            result.push(' ');
            i = j;
            continue;
        }

        // Match standalone `(...)` groups that contain `|` — these are enum
        // value lists, not subcommand alternatives.
        if chars[i] == '(' {
            let mut depth = 1;
            let mut j = i + 1;
            let mut has_pipe = false;
            while j < chars.len() && depth > 0 {
                if chars[j] == '(' {
                    depth += 1;
                } else if chars[j] == ')' {
                    depth -= 1;
                } else if chars[j] == '|' && depth == 1 {
                    has_pipe = true;
                }
                j += 1;
            }
            if has_pipe && depth == 0 {
                // This group had pipes inside parens — skip it entirely.
                result.push(' ');
                i = j;
                continue;
            }
            // No pipe inside parens — keep it.
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Joins all non-empty lines in a SYNOPSIS section into a single string so
/// that pipe-separated subcommand alternatives spanning multiple continuation
/// lines can be analyzed together.
fn join_synopsis_text(section: &ManSection) -> String {
    section
        .lines
        .iter()
        .map(|l| l.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn infer_value_type(token: &str) -> ValueType {
    let lower = token.to_ascii_lowercase();
    if lower.contains("file") || lower.contains("path") {
        ValueType::File
    } else if lower.contains("dir") {
        ValueType::Directory
    } else if lower.contains("url") {
        ValueType::Url
    } else if lower.contains("num") || lower.contains("count") {
        ValueType::Number
    } else {
        ValueType::String
    }
}

fn split_flag_aliases(token: &str) -> Vec<String> {
    token
        .split(|ch: char| ch == '|' || ch == ',')
        .map(str::trim)
        .filter(|part| !part.is_empty() && part.starts_with('-'))
        .map(|part| {
            // Expand --[no-]foo → --foo
            if let Some(rest) = part.strip_prefix("--[no-]") {
                format!("--{rest}")
            } else {
                part.to_string()
            }
        })
        .collect()
}

/// Strips trailing punctuation from a flag name that leaks through from man
/// page notation (e.g. `--exec-path[` → `--exec-path`, `--set-upstream-to.` →
/// `--set-upstream-to`).
fn normalize_flag_name(raw: &str) -> String {
    raw.trim_end_matches(|ch: char| matches!(ch, '[' | ']' | '<' | '>' | '.' | ','))
        .to_string()
}

/// Returns `true` when a flag name looks structurally valid.
///
/// Rejects garbage like `-)x`, `-S[<keyid`, `-m/-c/-C/-F).`, `-98`, and
/// other malformed short-flag artifacts from man-page notation.
fn is_valid_flag_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if name.starts_with("--") {
        // Long flag: body must start with a letter (rejects "---" and
        // ASCII art like "---o---O---P---Q"), only alphanumeric/hyphen
        // characters allowed.
        let body = &name[2..];
        !body.is_empty()
            && body
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic())
            && body
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    } else if name.starts_with('-') {
        // Short flag: `-` followed by 1-2 alphanumeric chars.
        let body = &name[1..];
        !body.is_empty()
            && body.len() <= 2
            && body.chars().all(|ch| ch.is_ascii_alphanumeric())
    } else {
        false
    }
}

fn looks_like_command_name(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        && token
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
}

fn is_placeholder_command_token(token: &str) -> bool {
    matches!(
        token,
        "command" | "commands" | "cmd" | "subcommand" | "option" | "options"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::IndexedLine;
    use crate::parser::strategies::man::rendered::sections::ManSection;

    #[test]
    fn test_parse_synopsis_flags_splits_pipe_aliases() {
        let section = ManSection {
            name: "SYNOPSIS".to_string(),
            start_line: 0,
            end_line: 0,
            lines: vec![IndexedLine {
                index: 0,
                text: "tool -p|--paginate|-P|--no-pager".to_string(),
            }],
        };

        let flags = parse_synopsis_flags(&section);
        assert!(flags.iter().any(|flag| flag.short.as_deref() == Some("-p")));
        assert!(flags.iter().any(|flag| flag.short.as_deref() == Some("-P")));
        assert!(
            flags
                .iter()
                .any(|flag| flag.long.as_deref() == Some("--paginate"))
        );
        assert!(
            flags
                .iter()
                .any(|flag| flag.long.as_deref() == Some("--no-pager"))
        );
        assert!(flags.iter().all(|flag| {
            flag.long
                .as_deref()
                .is_none_or(|long| long.starts_with("--"))
        }));
    }

    #[test]
    fn test_parse_synopsis_args_skips_normalized_flag_tokens() {
        let section = ManSection {
            name: "SYNOPSIS".to_string(),
            start_line: 0,
            end_line: 0,
            lines: vec![IndexedLine {
                index: 0,
                text: "tool {-h | --help} [-v]".to_string(),
            }],
        };

        let args = parse_synopsis_args(&section);
        assert!(args.iter().all(|arg| !arg.name.starts_with('-')));
    }

    #[test]
    fn test_parse_synopsis_subcommands_extracts_verb_alternatives() {
        let section = ManSection {
            name: "SYNOPSIS".to_string(),
            start_line: 0,
            end_line: 0,
            lines: vec![IndexedLine {
                index: 0,
                text: "apt-get install pkg... | remove pkg... | update | {-h | --help}"
                    .to_string(),
            }],
        };

        let subs = parse_synopsis_subcommands(&section);
        assert!(subs.iter().any(|sub| sub.name == "install"));
        assert!(subs.iter().any(|sub| sub.name == "remove"));
        assert!(subs.iter().any(|sub| sub.name == "update"));
        assert!(subs.iter().all(|sub| sub.name != "help"));
    }

    #[test]
    fn test_parse_synopsis_subcommands_multiline_apt_get() {
        // Simulates the real apt-get man page synopsis which spans multiple
        // continuation lines.
        let section = ManSection {
            name: "SYNOPSIS".to_string(),
            start_line: 0,
            end_line: 6,
            lines: vec![
                IndexedLine {
                    index: 0,
                    text: "apt-get [-sqdyfmubV] [-o=config_string] [-c=config_file]"
                        .to_string(),
                },
                IndexedLine {
                    index: 1,
                    text: "[-t=target_release] [-a=architecture] {update | upgrade |"
                        .to_string(),
                },
                IndexedLine {
                    index: 2,
                    text: "dselect-upgrade | dist-upgrade |".to_string(),
                },
                IndexedLine {
                    index: 3,
                    text: "install pkg [{=pkg_version_number | /target_release}]... |"
                        .to_string(),
                },
                IndexedLine {
                    index: 4,
                    text: "remove pkg... | purge pkg... |".to_string(),
                },
                IndexedLine {
                    index: 5,
                    text: "check | clean | autoclean | autoremove | {-v | --version} |"
                        .to_string(),
                },
                IndexedLine {
                    index: 6,
                    text: "{-h | --help}}".to_string(),
                },
            ],
        };

        let subs = parse_synopsis_subcommands(&section);
        let names: HashSet<String> = subs.iter().map(|s| s.name.clone()).collect();
        assert!(names.contains("update"), "missing update");
        assert!(names.contains("upgrade"), "missing upgrade");
        assert!(names.contains("dselect-upgrade"), "missing dselect-upgrade");
        assert!(names.contains("dist-upgrade"), "missing dist-upgrade");
        assert!(names.contains("install"), "missing install");
        assert!(names.contains("remove"), "missing remove");
        assert!(names.contains("purge"), "missing purge");
        assert!(names.contains("check"), "missing check");
        assert!(names.contains("clean"), "missing clean");
        assert!(names.contains("autoclean"), "missing autoclean");
        assert!(names.contains("autoremove"), "missing autoremove");
        // Flags and placeholders must not leak through
        assert!(!names.contains("help"), "help leaked");
        assert!(!names.contains("pkg"), "pkg placeholder leaked");
        assert!(!names.contains("version"), "version leaked");

        // Args should not include subcommand names
        let args = parse_synopsis_args(&section);
        let arg_names: HashSet<String> = args.iter().map(|a| a.name.clone()).collect();
        assert!(
            !arg_names.contains("update"),
            "subcommand update leaked to args"
        );
        assert!(
            !arg_names.contains("install"),
            "subcommand install leaked to args"
        );
        // pkg IS a legitimate positional arg
        assert!(arg_names.contains("pkg"), "pkg should be a positional arg");
    }
}
