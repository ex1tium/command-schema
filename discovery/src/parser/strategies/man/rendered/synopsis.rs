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
                let (name, inline_value) = alias
                    .split_once('=')
                    .map(|(head, _)| (head, true))
                    .unwrap_or((alias.as_str(), false));

                let mut schema = if name.starts_with("--") {
                    FlagSchema::boolean(None, Some(name))
                } else {
                    // Treat all single-dash forms as short-style flags to
                    // avoid invalid long names like "-foo".
                    FlagSchema::boolean(Some(name), None)
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

    for line in &section.lines {
        let trimmed = line.text.trim();
        if trimmed.is_empty() {
            continue;
        }

        for (idx, raw) in trimmed.split_whitespace().enumerate() {
            if raw.starts_with('-') {
                continue;
            }
            let bracketed = raw.contains('[') || raw.contains('<') || raw.contains('{');
            let token = normalize_synopsis_arg_token(raw);
            if token.is_empty() {
                continue;
            }
            if token.starts_with('-') {
                continue;
            }

            // Synopsis lines are usually "<command> [args...]"; avoid treating the
            // command token itself as a positional arg when unbracketed.
            if idx == 0 && !bracketed {
                continue;
            }

            let required = !raw.contains('[');
            let multiple = raw.contains("...");
            if !looks_like_arg_token(&token) {
                continue;
            }
            if all_subcommands.contains(&token.to_ascii_lowercase()) {
                continue;
            }

            let name = token.to_ascii_lowercase();
            if !seen.insert(name.clone()) {
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
        }
    }

    out
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
            '[' | ']' | '<' | '>' | '{' | '}' | '(' | ')' | ',' | ';'
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

    let root = line
        .split_whitespace()
        .next()
        .map(normalize_synopsis_arg_token)
        .unwrap_or_default();
    if !looks_like_command_name(&root) {
        return out;
    }
    let root_lower = root.to_ascii_lowercase();

    for segment in line.split('|') {
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
        .map(ToString::to_string)
        .collect()
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
