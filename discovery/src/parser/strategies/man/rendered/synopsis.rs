//! SYNOPSIS parser for rendered man pages.

use std::collections::HashSet;

use command_schema_core::{ArgSchema, FlagSchema, ValueType};

use crate::parser::ast::{ArgCandidate, FlagCandidate, SourceSpan};

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

            let (name, inline_value) = token
                .split_once('=')
                .map(|(head, _)| (head, true))
                .unwrap_or((token, false));

            let mut schema = if name.starts_with("--") {
                FlagSchema::boolean(None, Some(name))
            } else if name.len() == 2 {
                FlagSchema::boolean(Some(name), None)
            } else {
                FlagSchema::boolean(None, Some(name))
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

            idx += 1;
        }
    }

    out
}

pub fn parse_synopsis_args(section: &ManSection) -> Vec<ArgCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

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
