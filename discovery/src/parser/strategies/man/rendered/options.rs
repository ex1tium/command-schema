//! OPTIONS section parser for rendered man pages.

use std::collections::HashSet;

use command_schema_core::{FlagSchema, ValueType};

use crate::parser::ast::{FlagCandidate, SourceSpan};

use super::sections::ManSection;

pub fn parse_options_section(section: &ManSection) -> Vec<FlagCandidate> {
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
                "man-rendered-options",
                0.88,
            ));
        }
    }

    out
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
    let mut out = Vec::new();

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
            token
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch == '-')
        })
        || description.is_some_and(|desc| {
            desc.split_whitespace().any(|token| {
                token
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch == '-')
                    && token.len() > 1
            })
        });

    for part in parts {
        if !part.starts_with('-') {
            continue;
        }

        let (name, inline_value) = part
            .split_once('=')
            .map(|(head, _)| (head, true))
            .unwrap_or((part.as_str(), false));

        let mut schema = if name.starts_with("--") {
            FlagSchema::boolean(None, Some(name))
        } else if name.len() == 2 {
            FlagSchema::boolean(Some(name), None)
        } else {
            FlagSchema::boolean(None, Some(name))
        };

        if inline_value || has_value_hint {
            schema.takes_value = true;
            schema.value_type = infer_value_type(description.unwrap_or_default());
        }

        out.push(schema);
    }

    out
}

fn infer_value_type(description: &str) -> ValueType {
    let lower = description.to_ascii_lowercase();
    if lower.contains("file") || lower.contains("path") {
        ValueType::File
    } else if lower.contains("dir") {
        ValueType::Directory
    } else if lower.contains("url") {
        ValueType::Url
    } else if lower.contains("count") || lower.contains("number") {
        ValueType::Number
    } else {
        ValueType::String
    }
}
