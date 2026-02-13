//! COMMANDS/SUBCOMMANDS section parser for rendered man pages.

use std::collections::HashSet;

use command_schema_core::SubcommandSchema;

use crate::parser::ast::{SourceSpan, SubcommandCandidate};

use super::sections::ManSection;

pub fn parse_commands_section(section: &ManSection) -> Vec<SubcommandCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for line in &section.lines {
        let trimmed = line.text.trim();
        if trimmed.is_empty() || trimmed.starts_with('-') {
            continue;
        }

        let (candidate, description) = split_command_and_description(trimmed);
        if !looks_like_command_name(candidate) {
            // Indented lines without a valid command name are likely
            // continuation/description lines — skip them.
            continue;
        }

        let key = candidate.to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }

        let mut sub = SubcommandSchema::new(candidate);
        if let Some(desc) = description
            && !desc.is_empty()
        {
            sub.description = Some(desc.to_string());
        }

        out.push(SubcommandCandidate::from_schema(
            sub,
            SourceSpan::single(line.index),
            "man-rendered-commands",
            0.83,
        ));
    }

    out
}

fn split_command_and_description(line: &str) -> (&str, Option<&str>) {
    if let Some((left, right)) = line.split_once('\t') {
        return (left.trim(), Some(right.trim()));
    }
    if let Some((left, right)) = line.split_once("  ") {
        return (left.trim(), Some(right.trim()));
    }
    if let Some((left, right)) = line.split_once(" - ") {
        return (left.trim(), Some(right.trim()));
    }
    (line, None)
}

fn looks_like_command_name(value: &str) -> bool {
    super::super::looks_like_command_name(value.trim())
}
