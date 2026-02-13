//! OPTIONS section parser for rendered man pages.

use std::collections::HashSet;

use command_schema_core::FlagSchema;

use crate::parser::ast::{FlagCandidate, SourceSpan};
use crate::parser::strategies::man::infer_value_type;

use super::sections::ManSection;

pub fn parse_options_section(section: &ManSection) -> Vec<FlagCandidate> {
    parse_options_section_with_metadata(section, "man-rendered-options", 0.88)
}

pub fn parse_options_section_with_metadata(
    section: &ManSection,
    source: &'static str,
    confidence: f64,
) -> Vec<FlagCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for line in &section.lines {
        let trimmed = line.text.trim();
        if trimmed.is_empty() || !trimmed.starts_with('-') {
            continue;
        }

        let (definition, description) = split_definition_and_description(trimmed);
        let parsed = parse_flag_definition(definition, description);

        for mut flag in parsed {
            let key = flag.long.clone().or(flag.short.clone()).unwrap_or_default();
            if key.is_empty() || !seen.insert(key) {
                continue;
            }

            if let Some(desc) = description
                && !desc.is_empty()
            {
                flag.description = Some(desc.to_string());
            }

            out.push(FlagCandidate::from_schema(
                flag,
                SourceSpan::single(line.index),
                source,
                confidence,
            ));
        }
    }

    out
}

pub fn has_option_like_lines(section: &ManSection) -> bool {
    section
        .lines
        .iter()
        .any(|line| line.text.trim_start().starts_with('-'))
}

fn split_definition_and_description(line: &str) -> (&str, Option<&str>) {
    if let Some((left, right)) = line.split_once('\t') {
        return (left.trim(), Some(right.trim()));
    }
    if let Some((left, right)) = line.split_once("  ") {
        return (left.trim(), Some(right.trim()));
    }
    (line, None)
}

fn parse_flag_definition(definition: &str, description: Option<&str>) -> Vec<FlagSchema> {
    let parts = definition
        .split(|ch: char| ch == ',' || ch == '|' || ch.is_ascii_whitespace())
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            part.trim()
                .trim_matches(|ch: char| {
                    matches!(ch, '[' | ']' | '<' | '>' | '(' | ')' | '"' | '\'')
                })
                .to_string()
        })
        .collect::<Vec<_>>();

    let has_value_hint = definition.contains('=')
        || definition.split_whitespace().any(|token| {
            // Skip flag-like tokens (e.g. -C, --FOO) so uppercase flags
            // aren't mistaken for value placeholders.
            if token.trim_start_matches(|ch: char| {
                matches!(ch, '<' | '>' | '[' | ']' | '(' | ')' | ',' | ';')
            }).starts_with('-') {
                return false;
            }
            let normalized = token.trim_matches(|ch: char| {
                matches!(ch, '<' | '>' | '[' | ']' | '(' | ')' | ',' | ';')
            });
            !normalized.is_empty()
                && normalized.len() > 1
                && normalized
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch == '-')
        })
        || description.is_some_and(|desc| {
            desc.split_whitespace().any(|token| {
                if token.trim_start_matches(|ch: char| {
                    matches!(ch, '<' | '>' | '[' | ']' | '(' | ')' | ',' | ';')
                }).starts_with('-') {
                    return false;
                }
                let normalized = token.trim_matches(|ch: char| {
                    matches!(ch, '<' | '>' | '[' | ']' | '(' | ')' | ',' | ';')
                });
                !normalized.is_empty()
                    && normalized
                        .chars()
                        .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch == '-')
                    && normalized.len() > 1
            })
        });

    let mut first_short: Option<String> = None;
    let mut first_long: Option<String> = None;
    let mut has_inline_value = false;

    for part in parts {
        if !part.starts_with('-') {
            continue;
        }

        // Expand --[no-]foo → --foo (the positive form).
        let part = if let Some(rest) = part.strip_prefix("--[no-]") {
            format!("--{rest}")
        } else {
            part
        };

        let (name, inline_value) = if let Some((head, _)) = part.split_once('=') {
            (head, true)
        } else if let Some(pos) = part.find(|ch: char| ch == '<' || ch == '[' || ch == '(') {
            (&part[..pos], true)
        } else {
            (part.as_str(), false)
        };
        has_inline_value |= inline_value;

        let name = name
            .trim_end_matches(|ch: char| matches!(ch, ']' | '>' | '[' | '.' | ',' | '(' | ')'));

        if name.starts_with("--") {
            // Long flag: must have valid body — starts with a letter,
            // contains only alphanumeric/hyphen/underscore, and doesn't
            // start with another dash (rejects "---" and ASCII art).
            let body = &name[2..];
            if !body.is_empty()
                && body
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_alphabetic())
                && body
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
                && first_long.is_none()
            {
                first_long = Some(name.to_string());
            }
        } else {
            // Short flag: `-` followed by alphanumeric chars or common
            // symbolic flags like -? (help in some tools).
            let body = &name[1..];
            if !body.is_empty()
                && body
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '?')
                && first_short.is_none()
            {
                first_short = Some(name.to_string());
            }
        }
    }

    if first_short.is_none() && first_long.is_none() {
        return Vec::new();
    }

    let mut schema = FlagSchema::boolean(first_short.as_deref(), first_long.as_deref());
    if has_inline_value || has_value_hint {
        schema.takes_value = true;
        schema.value_type = infer_value_type(description.unwrap_or_default());
    }

    vec![schema]
}

#[cfg(test)]
mod tests {
    use super::*;
    use command_schema_core::ValueType;

    #[test]
    fn test_parse_flag_definition_single_dash_multi_char_is_short() {
        let flags = parse_flag_definition("-eany", None);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].short.as_deref(), Some("-eany"));
        assert!(flags[0].long.is_none());
    }

    #[test]
    fn test_parse_flag_definition_merges_alias_pair_into_single_schema() {
        let flags = parse_flag_definition("-a, --all", Some("Show all entries"));
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].short.as_deref(), Some("-a"));
        assert_eq!(flags[0].long.as_deref(), Some("--all"));
    }

    #[test]
    fn test_parse_flag_definition_merges_aliases_with_value_once() {
        let flags = parse_flag_definition("-o, --output=FILE", Some("Output file path"));
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].short.as_deref(), Some("-o"));
        assert_eq!(flags[0].long.as_deref(), Some("--output"));
        assert!(flags[0].takes_value);
        assert_eq!(flags[0].value_type, ValueType::File);
    }

    #[test]
    fn test_parse_flag_definition_detects_bracketed_value_placeholders() {
        // Bracketed <FILE> should be recognized as a value hint
        let flags = parse_flag_definition("--config <FILE>", Some("Config file"));
        assert_eq!(flags.len(), 1);
        assert!(flags[0].takes_value);
        assert_eq!(flags[0].value_type, ValueType::File);

        // Bracketed [FILE] should also be recognized
        let flags = parse_flag_definition("--config [FILE]", Some("Config file"));
        assert_eq!(flags.len(), 1);
        assert!(flags[0].takes_value);
    }
}
