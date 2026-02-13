//! Legacy man macro parser.

use std::collections::HashSet;

use command_schema_core::{ArgSchema, FlagSchema, SubcommandSchema};

use crate::parser::ast::{ArgCandidate, FlagCandidate, SourceSpan, SubcommandCandidate};
use crate::parser::strategies::man::infer_value_type;

use super::lexer::Token;

/// Parsed representation of a legacy `man`-macro document.
///
/// `title` and `section` are optional metadata from `.TH`; `sections` preserves
/// document order and owns all extracted content.
#[derive(Debug, Clone, Default)]
pub struct ManDocument {
    /// Command/manual title from `.TH`, when present.
    pub title: Option<String>,
    /// Manual section identifier from `.TH`, when present.
    pub section: Option<String>,
    /// Ordered top-level sections extracted from `.SH` boundaries.
    pub sections: Vec<ManSection>,
}

/// A top-level legacy-man section and its parsed elements.
#[derive(Debug, Clone, Default)]
pub struct ManSection {
    /// Canonical section name (typically uppercase).
    pub name: String,
    /// Elements parsed from the section body in source order.
    pub content: Vec<ManElement>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum ManElement {
    /// A `.TP`-style definition list row (`tag`, `description`, source `line`).
    TaggedParagraph {
        tag: String,
        description: String,
        line: usize,
    },
    /// An `.IP`-style indented row with optional `tag`, body `text`, and `line`.
    IndentedParagraph {
        tag: Option<String>,
        text: String,
        line: usize,
    },
    /// Free text content and its source `line`.
    Text { value: String, line: usize },
    /// Paragraph boundary marker with source `line`.
    Paragraph { line: usize },
}

/// Parses legacy `man` macro tokens into a structured [`ManDocument`].
///
/// Unknown macros are preserved as text-like content; the function never
/// returns an error.
pub fn parse_man_source(tokens: &[Token]) -> ManDocument {
    let mut doc = ManDocument::default();
    let mut current_section = "UNKNOWN".to_string();

    let mut pending_tagged_start: Option<usize> = None;
    let mut pending_tag_line: Option<usize> = None;
    let mut tagged_tag = String::new();

    for token in tokens {
        match token {
            Token::Macro { name, args, line } => {
                let macro_name = name.to_ascii_uppercase();
                match macro_name.as_str() {
                    "TH" => {
                        doc.title = args.first().cloned();
                        doc.section = args.get(1).cloned();
                    }
                    "SH" => {
                        flush_tagged_paragraph(
                            &mut doc,
                            &current_section,
                            &mut pending_tagged_start,
                            &mut pending_tag_line,
                            &mut tagged_tag,
                            String::new(),
                        );

                        let title = args.join(" ").trim().trim_matches('"').to_ascii_uppercase();
                        if !title.is_empty() {
                            current_section = title;
                        }
                        ensure_section(&mut doc, &current_section);
                    }
                    "TP" => {
                        flush_tagged_paragraph(
                            &mut doc,
                            &current_section,
                            &mut pending_tagged_start,
                            &mut pending_tag_line,
                            &mut tagged_tag,
                            String::new(),
                        );
                        pending_tagged_start = Some(*line);
                        pending_tag_line = Some(*line);
                        tagged_tag.clear();
                    }
                    "IP" => {
                        flush_tagged_paragraph(
                            &mut doc,
                            &current_section,
                            &mut pending_tagged_start,
                            &mut pending_tag_line,
                            &mut tagged_tag,
                            String::new(),
                        );

                        let tag = args
                            .first()
                            .map(|value| value.trim_matches('"').to_string());
                        let text = if args.len() > 1 {
                            args[1..].join(" ").trim_matches('"').to_string()
                        } else {
                            String::new()
                        };
                        push_element(
                            &mut doc,
                            &current_section,
                            ManElement::IndentedParagraph {
                                tag,
                                text,
                                line: *line,
                            },
                        );
                    }
                    "PP" | "P" => {
                        flush_tagged_paragraph(
                            &mut doc,
                            &current_section,
                            &mut pending_tagged_start,
                            &mut pending_tag_line,
                            &mut tagged_tag,
                            String::new(),
                        );
                        push_element(
                            &mut doc,
                            &current_section,
                            ManElement::Paragraph { line: *line },
                        );
                    }
                    "B" | "I" | "BR" | "BI" | "RB" | "RI" => {
                        let rendered = args.join(" ").trim().to_string();
                        if rendered.is_empty() {
                            continue;
                        }

                        if pending_tag_line.is_some() && tagged_tag.is_empty() {
                            tagged_tag = rendered;
                            pending_tag_line = None;
                        } else if pending_tagged_start.is_some() {
                            flush_tagged_paragraph(
                                &mut doc,
                                &current_section,
                                &mut pending_tagged_start,
                                &mut pending_tag_line,
                                &mut tagged_tag,
                                rendered,
                            );
                        } else {
                            push_element(
                                &mut doc,
                                &current_section,
                                ManElement::Text {
                                    value: rendered,
                                    line: *line,
                                },
                            );
                        }
                    }
                    _ => {
                        if !args.is_empty() {
                            let rendered = args.join(" ").trim().to_string();
                            if pending_tagged_start.is_some() {
                                flush_tagged_paragraph(
                                    &mut doc,
                                    &current_section,
                                    &mut pending_tagged_start,
                                    &mut pending_tag_line,
                                    &mut tagged_tag,
                                    rendered,
                                );
                            } else {
                                push_element(
                                    &mut doc,
                                    &current_section,
                                    ManElement::Text {
                                        value: rendered,
                                        line: *line,
                                    },
                                );
                            }
                        }
                    }
                }
            }
            Token::Text { value, line } => {
                if value.trim().is_empty() {
                    continue;
                }

                if pending_tag_line.is_some() && tagged_tag.is_empty() {
                    tagged_tag = value.trim().to_string();
                    pending_tag_line = None;
                    continue;
                }

                if pending_tagged_start.is_some() {
                    flush_tagged_paragraph(
                        &mut doc,
                        &current_section,
                        &mut pending_tagged_start,
                        &mut pending_tag_line,
                        &mut tagged_tag,
                        value.trim().to_string(),
                    );
                    continue;
                }

                push_element(
                    &mut doc,
                    &current_section,
                    ManElement::Text {
                        value: value.trim().to_string(),
                        line: *line,
                    },
                );
            }
            Token::Newline { .. } => {}
        }
    }

    flush_tagged_paragraph(
        &mut doc,
        &current_section,
        &mut pending_tagged_start,
        &mut pending_tag_line,
        &mut tagged_tag,
        String::new(),
    );

    doc
}

/// Extracts flag candidates from parsed legacy-man sections.
///
/// Primarily reads `OPTIONS`/`SYNOPSIS` tagged and indented paragraphs.
pub fn extract_flags_from_man(doc: &ManDocument) -> Vec<FlagCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for section in &doc.sections {
        let upper = section.name.to_ascii_uppercase();
        if !upper.contains("OPTION") && !upper.contains("SYNOPSIS") {
            continue;
        }

        for element in &section.content {
            match element {
                ManElement::TaggedParagraph {
                    tag,
                    description,
                    line,
                }
                | ManElement::IndentedParagraph {
                    tag: Some(tag),
                    text: description,
                    line,
                } => {
                    for flag in parse_flag_defs(tag, description) {
                        let key = flag.long.clone().or(flag.short.clone()).unwrap_or_default();
                        if key.is_empty() || !seen.insert(key) {
                            continue;
                        }
                        out.push(FlagCandidate::from_schema(
                            flag,
                            SourceSpan::single(*line),
                            "man-roff-man-options",
                            0.94,
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    out
}

/// Extracts positional argument candidates from `SYNOPSIS` content.
///
/// Synopsis text is tokenized heuristically and deduplicated by lowercase name.
/// Leading command tokens (derived from `.TH` title or the first synopsis word)
/// are stripped so multi-word commands like `git rebase` don't leak as args.
pub fn extract_args_from_man(doc: &ManDocument) -> Vec<ArgCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    // Build a set of command tokens to skip (e.g. {"git", "rebase"}).
    let command_tokens = derive_command_tokens(doc);

    for section in &doc.sections {
        let upper = section.name.to_ascii_uppercase();
        if !upper.contains("SYNOPSIS") {
            continue;
        }

        for element in &section.content {
            let (text, line) = match element {
                ManElement::Text { value, line } => (value.as_str(), *line),
                ManElement::IndentedParagraph { text, line, .. } => (text.as_str(), *line),
                ManElement::TaggedParagraph {
                    tag,
                    description,
                    line,
                } => {
                    let combined = format!("{tag} {description}");
                    for arg in
                        parse_args_from_synopsis(&combined, *line, &mut seen, &command_tokens)
                    {
                        out.push(arg);
                    }
                    continue;
                }
                _ => continue,
            };

            for arg in parse_args_from_synopsis(text, line, &mut seen, &command_tokens) {
                out.push(arg);
            }
        }
    }

    out
}

/// Derives the set of command-name tokens to skip from synopsis text.
///
/// Uses the `.TH` title (lowercased) plus leading unbracketed tokens from the
/// first synopsis text element. For `git-rebase`, this yields `{"git-rebase",
/// "git", "rebase"}`.
fn derive_command_tokens(doc: &ManDocument) -> HashSet<String> {
    let mut tokens = HashSet::new();

    // From .TH title
    if let Some(ref title) = doc.title {
        let lower = title.to_ascii_lowercase();
        // Split hyphenated titles: "git-rebase" → {"git-rebase", "git", "rebase"}
        tokens.insert(lower.clone());
        for part in lower.split('-') {
            if !part.is_empty() {
                tokens.insert(part.to_string());
            }
        }
    }

    // From first synopsis text element: take leading unbracketed words
    for section in &doc.sections {
        if !section.name.to_ascii_uppercase().contains("SYNOPSIS") {
            continue;
        }
        for element in &section.content {
            let text = match element {
                ManElement::Text { value, .. } => value.as_str(),
                ManElement::IndentedParagraph { text, .. } => text.as_str(),
                ManElement::TaggedParagraph { tag, .. } => tag.as_str(),
                _ => continue,
            };
            for word in text.split_whitespace() {
                if word.starts_with('-')
                    || word.contains('[')
                    || word.contains('<')
                    || word.contains('{')
                    || word.contains('(')
                {
                    break;
                }
                let lower = word.to_ascii_lowercase();
                tokens.insert(lower.clone());
                for part in lower.split('-') {
                    if !part.is_empty() {
                        tokens.insert(part.to_string());
                    }
                }
            }
            if !tokens.is_empty() {
                return tokens;
            }
        }
    }

    tokens
}

/// Extracts subcommand candidates from `COMMANDS`-like sections.
///
/// Tagged definitions provide command names and optional descriptions.
pub fn extract_subcommands_from_man(doc: &ManDocument) -> Vec<SubcommandCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for section in &doc.sections {
        let upper = section.name.to_ascii_uppercase();
        if !upper.contains("COMMAND") {
            continue;
        }

        for element in &section.content {
            let (candidate, line, desc) = match element {
                ManElement::TaggedParagraph {
                    tag,
                    description,
                    line,
                } => (tag.as_str(), *line, Some(description.as_str())),
                ManElement::IndentedParagraph {
                    tag: Some(tag),
                    text,
                    line,
                } => (tag.as_str(), *line, Some(text.as_str())),
                _ => continue,
            };

            // Split on commas first to handle aliases like "clone, cl",
            // then trim whitespace/punctuation and pick the first valid name.
            let token = candidate
                .split(',')
                .map(|s| s.trim())
                .map(|s| {
                    s.split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .trim_end_matches(|ch: char| ch.is_ascii_punctuation() && ch != '-' && ch != '_')
                })
                .find(|t| looks_like_command_name(t))
                .unwrap_or_default();
            if token.is_empty() || !seen.insert(token.to_ascii_lowercase()) {
                continue;
            }

            let mut sub = SubcommandSchema::new(token);
            if let Some(description) = desc.and_then(clean_description) {
                sub.description = Some(description);
            }
            out.push(SubcommandCandidate::from_schema(
                sub,
                SourceSpan::single(line),
                "man-roff-man-commands",
                0.91,
            ));
        }
    }

    out
}

fn ensure_section<'a>(doc: &'a mut ManDocument, name: &str) -> &'a mut ManSection {
    if let Some(index) = doc.sections.iter().position(|section| section.name == name) {
        return &mut doc.sections[index];
    }
    doc.sections.push(ManSection {
        name: name.to_string(),
        content: Vec::new(),
    });
    doc.sections.last_mut().expect("section was just inserted")
}

fn push_element(doc: &mut ManDocument, section_name: &str, element: ManElement) {
    let section = ensure_section(doc, section_name);
    section.content.push(element);
}

fn flush_tagged_paragraph(
    doc: &mut ManDocument,
    section: &str,
    pending_start: &mut Option<usize>,
    pending_tag_line: &mut Option<usize>,
    pending_tag: &mut String,
    description: String,
) {
    if let Some(line) = pending_start.take() {
        let tag = std::mem::take(pending_tag);
        if !tag.trim().is_empty() {
            push_element(
                doc,
                section,
                ManElement::TaggedParagraph {
                    tag: tag.trim().to_string(),
                    description: description.trim().to_string(),
                    line,
                },
            );
        }
    }
    *pending_tag_line = None;
}

fn parse_flag_defs(definition: &str, description: &str) -> Vec<FlagSchema> {
    let mut parts = definition
        .split(|ch: char| ch == ',' || ch == '|' || ch.is_ascii_whitespace())
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            part.trim()
                .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '[' | ']' | '(' | ')'))
                .to_string()
        })
        .collect::<Vec<_>>();

    parts.retain(|part| part.starts_with('-'));

    let value_hint = definition.contains('=')
        || definition.split_whitespace().any(|token| {
            let normalized = token.trim_matches(|ch: char| {
                matches!(ch, '<' | '>' | '[' | ']' | '(' | ')' | ',' | ';')
            });
            !normalized.is_empty()
                && normalized
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch == '-')
                && normalized.len() > 1
        })
        || description.contains("=")
        || description.split_whitespace().any(|token| {
            let normalized = token.trim_matches(|ch: char| {
                matches!(ch, '<' | '>' | '[' | ']' | '(' | ')' | ',' | ';')
            });
            !normalized.is_empty()
                && normalized
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch == '-')
                && normalized.len() > 1
        });

    let mut first_short: Option<String> = None;
    let mut first_long: Option<String> = None;
    let mut has_inline_value = false;

    for part in parts {
        let (raw_name, inline_value) = part
            .split_once('=')
            .map(|(name, _)| (name, true))
            .unwrap_or((part.as_str(), false));
        has_inline_value |= inline_value;

        if raw_name.starts_with("--") {
            if first_long.is_none() {
                first_long = Some(raw_name.to_string());
            }
        } else {
            // Treat all single-dash forms as short-style flags to avoid
            // generating invalid long names like "-foo".
            if first_short.is_none() {
                first_short = Some(raw_name.to_string());
            }
        }
    }

    if first_short.is_none() && first_long.is_none() {
        return Vec::new();
    }

    let mut schema = FlagSchema::boolean(first_short.as_deref(), first_long.as_deref());
    if has_inline_value || value_hint {
        schema.takes_value = true;
        schema.value_type = infer_value_type(description);
    }
    if let Some(clean) = clean_description(description) {
        schema.description = Some(clean);
    }

    vec![schema]
}

fn parse_args_from_synopsis(
    text: &str,
    line: usize,
    seen: &mut HashSet<String>,
    command_tokens: &HashSet<String>,
) -> Vec<ArgCandidate> {
    let mut out = Vec::new();
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut idx = 0;

    while idx < words.len() {
        let raw = words[idx];

        // Skip flag tokens and their value placeholders.
        if raw.starts_with('-') {
            if idx + 1 < words.len() {
                let next = words[idx + 1];
                let next_norm = normalize_synopsis_arg_token(next);
                if !next.starts_with('-')
                    && (next.contains('<')
                        || next.contains('>')
                        || next.starts_with('[')
                        || next.ends_with(']')
                        || (!next_norm.is_empty()
                            && next_norm
                                .chars()
                                .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch == '-'))
                        // Bare lowercase word after an unbracketed flag
                        // (e.g. "label" in `--label label`): treat as
                        // flag value when the flag has no inline value.
                        || (!raw.contains('=')
                            && !next.contains('[')
                            && !next.contains('<')
                            && !next.contains('{')
                            && !next_norm.is_empty()
                            && next_norm
                                .chars()
                                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')))
                {
                    idx += 2;
                    continue;
                }
            }
            idx += 1;
            continue;
        }

        let bracketed = raw.contains('[') || raw.contains('<') || raw.contains('{');
        let multiple = raw.contains("...");
        let token = normalize_synopsis_arg_token(raw);
        if token.is_empty() {
            idx += 1;
            continue;
        }

        let token_lower = token.to_ascii_lowercase();

        // Skip command name tokens at any position.
        if command_tokens.contains(&token_lower) && !bracketed {
            idx += 1;
            continue;
        }

        let required = !raw.contains('[');
        if !looks_like_synopsis_arg_token(&token) {
            idx += 1;
            continue;
        }

        if !seen.insert(token_lower.clone()) {
            idx += 1;
            continue;
        }

        let value_type = infer_value_type(&token);
        let mut schema = if required {
            ArgSchema::required(&token_lower, value_type)
        } else {
            ArgSchema::optional(&token_lower, value_type)
        };
        schema.multiple = multiple;
        out.push(ArgCandidate::from_schema(
            schema,
            SourceSpan::single(line),
            "man-roff-man-synopsis",
            0.92,
        ));
        idx += 1;
    }

    out
}

fn normalize_synopsis_arg_token(raw: &str) -> String {
    raw.trim_matches(|ch: char| {
        matches!(
            ch,
            '[' | ']' | '<' | '>' | '{' | '}' | '"' | '\'' | ',' | ';'
        )
    })
    .trim_end_matches("...")
    .trim()
    .to_string()
}

fn looks_like_synopsis_arg_token(token: &str) -> bool {
    // Must be more than 1 character (rejects bare "a", "c").
    if token.len() <= 1 {
        return false;
    }
    // Reject pure version numbers: only digits and dots (e.g. "5.004").
    if token.chars().all(|ch| ch.is_ascii_digit() || ch == '.') {
        return false;
    }
    !token.is_empty()
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && token.chars().any(|ch| ch.is_ascii_alphanumeric())
}

fn clean_description(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn looks_like_command_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        && value
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::parse_flag_defs;
    use command_schema_core::ValueType;

    #[test]
    fn test_parse_flag_defs_merges_alias_pair_into_single_schema() {
        let flags = parse_flag_defs("-a, --all", "Show all entries");
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].short.as_deref(), Some("-a"));
        assert_eq!(flags[0].long.as_deref(), Some("--all"));
        assert_eq!(flags[0].description.as_deref(), Some("Show all entries"));
    }

    #[test]
    fn test_parse_flag_defs_merges_aliases_with_value_once() {
        let flags = parse_flag_defs("-o, --output=FILE", "Write FILE to disk");
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].short.as_deref(), Some("-o"));
        assert_eq!(flags[0].long.as_deref(), Some("--output"));
        assert!(flags[0].takes_value);
        assert_eq!(flags[0].value_type, ValueType::File);
    }
}
