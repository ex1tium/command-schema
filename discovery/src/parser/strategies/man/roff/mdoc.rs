//! mdoc macro parser.

use std::collections::HashSet;

use command_schema_core::{ArgSchema, FlagSchema, SubcommandSchema, ValueType};

use crate::parser::ast::{ArgCandidate, FlagCandidate, SourceSpan, SubcommandCandidate};

use super::lexer::Token;

/// Parsed representation of an `mdoc` document.
///
/// `title` and `section` come from `.Dt`; `sections` keeps parsed content in
/// document order.
#[derive(Debug, Clone, Default)]
pub struct MdocDocument {
    /// Document title from `.Dt`, when present.
    pub title: Option<String>,
    /// Manual section from `.Dt`, when present.
    pub section: Option<String>,
    /// Ordered top-level sections derived from `.Sh`.
    pub sections: Vec<MdocSection>,
}

/// A top-level `mdoc` section and its parsed elements.
#[derive(Debug, Clone, Default)]
pub struct MdocSection {
    /// Section heading name (typically uppercase after normalization).
    pub name: String,
    /// Elements captured from the section in source order.
    pub content: Vec<MdocElement>,
}

/// Token types produced by the mdoc parser, representing flags, positional
/// arguments, subcommands, free text, and paragraph boundaries. Each variant
/// carries a source `line` index for diagnostics and span tracking.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum MdocElement {
    /// A flag token with optionality and source line.
    Flag {
        name: String,
        optional: bool,
        line: usize,
    },
    /// A positional argument token with optionality and source line.
    Arg {
        name: String,
        optional: bool,
        line: usize,
    },
    /// A subcommand token and source line.
    Command { name: String, line: usize },
    /// Free text content and source line.
    Text { value: String, line: usize },
    /// Paragraph boundary marker and source line.
    Paragraph { line: usize },
}

/// Parses `mdoc` tokens into a structured [`MdocDocument`].
///
/// Unknown macros are retained as text-like elements; the parser does not
/// return errors.
pub fn parse_mdoc_source(tokens: &[Token]) -> MdocDocument {
    let mut doc = MdocDocument::default();
    let mut current_section = "UNKNOWN".to_string();
    let mut list_depth = 0usize;

    for token in tokens {
        match token {
            Token::Macro { name, args, line } => {
                let macro_name = name.to_ascii_lowercase();
                match macro_name.as_str() {
                    "dt" => {
                        doc.title = args.first().cloned();
                        doc.section = args.get(1).cloned();
                    }
                    "sh" => {
                        let title = args.join(" ").trim().to_ascii_uppercase();
                        if !title.is_empty() {
                            current_section = title;
                        }
                        ensure_section(&mut doc, &current_section);
                    }
                    "ss" => {
                        if !args.is_empty() {
                            push_element(
                                &mut doc,
                                &current_section,
                                MdocElement::Text {
                                    value: args.join(" "),
                                    line: *line,
                                },
                            );
                        }
                    }
                    "bl" => {
                        list_depth = list_depth.saturating_add(1);
                    }
                    "el" => {
                        list_depth = list_depth.saturating_sub(1);
                    }
                    "it" => {
                        if list_depth == 0 {
                            continue;
                        }
                        for element in parse_it_elements(args, *line) {
                            push_element(&mut doc, &current_section, element);
                        }
                    }
                    "fl" => {
                        let Some(raw) = args.first() else {
                            continue;
                        };
                        if let Some(flag_name) = normalize_flag_name(raw) {
                            push_element(
                                &mut doc,
                                &current_section,
                                MdocElement::Flag {
                                    name: flag_name,
                                    optional: false,
                                    line: *line,
                                },
                            );
                        }
                    }
                    "ar" => {
                        if let Some(arg) = args.first().cloned() {
                            push_element(
                                &mut doc,
                                &current_section,
                                MdocElement::Arg {
                                    name: normalize_arg_name(&arg),
                                    optional: false,
                                    line: *line,
                                },
                            );
                        }
                    }
                    "cm" | "ic" => {
                        if let Some(cmd) = args.first().cloned() {
                            push_element(
                                &mut doc,
                                &current_section,
                                MdocElement::Command {
                                    name: cmd.trim().to_string(),
                                    line: *line,
                                },
                            );
                        }
                    }
                    "op" => {
                        if args.is_empty() {
                            continue;
                        }
                        let head = args[0].to_ascii_lowercase();
                        if head == "fl" {
                            if let Some(raw) = args.get(1)
                                && let Some(flag_name) = normalize_flag_name(raw)
                            {
                                push_element(
                                    &mut doc,
                                    &current_section,
                                    MdocElement::Flag {
                                        name: flag_name,
                                        optional: true,
                                        line: *line,
                                    },
                                );
                            }
                        } else if head == "ar" {
                            if let Some(arg) = args.get(1) {
                                push_element(
                                    &mut doc,
                                    &current_section,
                                    MdocElement::Arg {
                                        name: normalize_arg_name(arg),
                                        optional: true,
                                        line: *line,
                                    },
                                );
                            }
                        }
                    }
                    "nd" => {
                        if !args.is_empty() {
                            push_element(
                                &mut doc,
                                &current_section,
                                MdocElement::Text {
                                    value: args.join(" ").trim().to_string(),
                                    line: *line,
                                },
                            );
                        }
                    }
                    "pp" => {
                        push_element(
                            &mut doc,
                            &current_section,
                            MdocElement::Paragraph { line: *line },
                        );
                    }
                    _ => {
                        if !args.is_empty() {
                            push_element(
                                &mut doc,
                                &current_section,
                                MdocElement::Text {
                                    value: args.join(" "),
                                    line: *line,
                                },
                            );
                        }
                    }
                }
            }
            Token::Text { value, line } => {
                if !value.trim().is_empty() {
                    push_element(
                        &mut doc,
                        &current_section,
                        MdocElement::Text {
                            value: value.trim().to_string(),
                            line: *line,
                        },
                    );
                }
            }
            Token::Newline { .. } => {}
        }
    }

    doc
}

/// Extracts flag candidates from parsed `mdoc` sections.
///
/// Flags are sourced from option macros and list-item content.
pub fn extract_flags_from_mdoc(doc: &MdocDocument) -> Vec<FlagCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for section in &doc.sections {
        let upper = section.name.to_ascii_uppercase();
        let confidence = if upper.contains("OPTION") { 0.95 } else { 0.9 };

        for (idx, element) in section.content.iter().enumerate() {
            let MdocElement::Flag {
                name,
                optional: _,
                line,
            } = element
            else {
                continue;
            };

            let Some(flag) = parse_flag_from_name(name) else {
                continue;
            };
            let key = flag.long.clone().or(flag.short.clone()).unwrap_or_default();
            if key.is_empty() || !seen.insert(key) {
                continue;
            }

            let takes_value = matches!(section.content.get(idx + 1), Some(MdocElement::Arg { .. }));
            let mut schema = flag;
            schema.takes_value = takes_value;
            if takes_value {
                schema.value_type = ValueType::String;
            }
            schema.description = next_text_description(&section.content, idx + 1);

            out.push(FlagCandidate::from_schema(
                schema,
                SourceSpan::single(*line),
                "man-roff-mdoc-options",
                confidence,
            ));
        }
    }

    out
}

/// Extracts positional argument candidates from `SYNOPSIS`/`USAGE` sections.
///
/// Argument names are normalized and deduplicated by lowercase key.
pub fn extract_args_from_mdoc(doc: &MdocDocument) -> Vec<ArgCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for section in &doc.sections {
        let upper = section.name.to_ascii_uppercase();
        if !upper.contains("SYNOPSIS") && !upper.contains("USAGE") {
            continue;
        }

        for element in &section.content {
            let MdocElement::Arg {
                name,
                optional,
                line,
            } = element
            else {
                continue;
            };

            let arg_name = normalize_arg_name(name);
            if arg_name.is_empty() || !seen.insert(arg_name.clone()) {
                continue;
            }

            let value_type = infer_value_type(&arg_name);
            let schema = if *optional {
                ArgSchema::optional(&arg_name, value_type)
            } else {
                ArgSchema::required(&arg_name, value_type)
            };

            out.push(ArgCandidate::from_schema(
                schema,
                SourceSpan::single(*line),
                "man-roff-mdoc-synopsis",
                0.93,
            ));
        }
    }

    out
}

/// Extracts subcommand candidates from `COMMANDS`-like sections.
///
/// Command entries are deduplicated case-insensitively.
pub fn extract_subcommands_from_mdoc(doc: &MdocDocument) -> Vec<SubcommandCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for section in &doc.sections {
        let upper = section.name.to_ascii_uppercase();
        if !upper.contains("COMMAND") {
            continue;
        }

        for element in &section.content {
            let MdocElement::Command { name, line } = element else {
                continue;
            };
            if !looks_like_command_name(name) || !seen.insert(name.to_ascii_lowercase()) {
                continue;
            }

            let sub = SubcommandSchema::new(name);
            out.push(SubcommandCandidate::from_schema(
                sub,
                SourceSpan::single(*line),
                "man-roff-mdoc-commands",
                0.92,
            ));
        }
    }

    out
}

fn ensure_section<'a>(doc: &'a mut MdocDocument, name: &str) -> &'a mut MdocSection {
    if let Some(index) = doc.sections.iter().position(|section| section.name == name) {
        return &mut doc.sections[index];
    }
    doc.sections.push(MdocSection {
        name: name.to_string(),
        content: Vec::new(),
    });
    doc.sections.last_mut().expect("section was just inserted")
}

fn push_element(doc: &mut MdocDocument, section_name: &str, element: MdocElement) {
    let section = ensure_section(doc, section_name);
    section.content.push(element);
}

fn normalize_flag_name(raw: &str) -> Option<String> {
    let token = raw
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | ',' | '[' | ']' | '(' | ')' | '{' | '}'));
    if token.is_empty() {
        return None;
    }

    if token.starts_with("--") {
        return Some(token.to_string());
    }
    if token.starts_with('-') {
        return Some(token.to_string());
    }
    if token.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Some(format!("-{token}"));
    }

    None
}

fn parse_flag_from_name(name: &str) -> Option<FlagSchema> {
    if name.starts_with("--") {
        return Some(FlagSchema::boolean(None, Some(name)));
    }
    if name.starts_with('-') {
        // Treat all single-dash forms as short-style flags to avoid
        // generating invalid long names like "-foo".
        return Some(FlagSchema::boolean(Some(name), None));
    }
    None
}

fn normalize_arg_name(raw: &str) -> String {
    raw.trim()
        .trim_matches(|ch: char| matches!(ch, '<' | '>' | '[' | ']' | '{' | '}' | '"' | '\''))
        .to_ascii_lowercase()
}

fn infer_value_type(name: &str) -> ValueType {
    let lower = name.to_ascii_lowercase();
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

fn next_text_description(elements: &[MdocElement], start: usize) -> Option<String> {
    for element in elements.iter().skip(start) {
        match element {
            MdocElement::Text { value, .. } if !value.trim().is_empty() => {
                return Some(value.trim().to_string());
            }
            MdocElement::Flag { .. } | MdocElement::Paragraph { .. } => return None,
            _ => {}
        }
    }
    None
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

fn parse_it_elements(args: &[String], line: usize) -> Vec<MdocElement> {
    let mut out = Vec::new();
    let mut idx = 0usize;
    let item_optional = args
        .first()
        .is_some_and(|head| head.eq_ignore_ascii_case("op"));
    if item_optional {
        idx = 1;
    }
    let mut pending_optional = false;

    while idx < args.len() {
        let head = args[idx].to_ascii_lowercase();
        match head.as_str() {
            "op" => {
                pending_optional = true;
                idx += 1;
                continue;
            }
            "fl" => {
                if let Some(raw) = args.get(idx + 1)
                    && let Some(name) = normalize_flag_name(raw)
                {
                    out.push(MdocElement::Flag {
                        name,
                        optional: item_optional || pending_optional,
                        line,
                    });
                }
                idx = idx.saturating_add(2);
            }
            "ar" => {
                if let Some(raw) = args.get(idx + 1) {
                    let name = normalize_arg_name(raw);
                    if !name.is_empty() {
                        out.push(MdocElement::Arg {
                            name,
                            optional: item_optional || pending_optional,
                            line,
                        });
                    }
                }
                idx = idx.saturating_add(2);
            }
            "cm" | "ic" => {
                if let Some(raw) = args.get(idx + 1) {
                    let name = raw.trim().to_string();
                    if !name.is_empty() {
                        out.push(MdocElement::Command { name, line });
                    }
                }
                idx = idx.saturating_add(2);
            }
            _ => {
                let raw = args[idx].trim();
                if raw.starts_with('-')
                    && let Some(name) = normalize_flag_name(raw)
                {
                    out.push(MdocElement::Flag {
                        name,
                        optional: item_optional || pending_optional,
                        line,
                    });
                }
                idx += 1;
            }
        }
        pending_optional = false;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::strategies::man::roff::lexer::Token;

    #[test]
    fn test_parse_mdoc_it_list_emits_flag_and_subcommand_candidates() {
        let tokens = vec![
            Token::Macro {
                name: "Sh".to_string(),
                args: vec!["OPTIONS".to_string()],
                line: 0,
            },
            Token::Macro {
                name: "Bl".to_string(),
                args: vec!["-tag".to_string()],
                line: 1,
            },
            Token::Macro {
                name: "It".to_string(),
                args: vec![
                    "Fl".to_string(),
                    "v".to_string(),
                    "Ar".to_string(),
                    "file".to_string(),
                ],
                line: 2,
            },
            Token::Macro {
                name: "El".to_string(),
                args: vec![],
                line: 3,
            },
            Token::Macro {
                name: "Sh".to_string(),
                args: vec!["COMMANDS".to_string()],
                line: 4,
            },
            Token::Macro {
                name: "Bl".to_string(),
                args: vec!["-tag".to_string()],
                line: 5,
            },
            Token::Macro {
                name: "It".to_string(),
                args: vec!["Cm".to_string(), "sync".to_string()],
                line: 6,
            },
            Token::Macro {
                name: "El".to_string(),
                args: vec![],
                line: 7,
            },
        ];

        let doc = parse_mdoc_source(&tokens);
        let flags = extract_flags_from_mdoc(&doc);
        assert!(
            flags
                .iter()
                .any(|candidate| candidate.short.as_deref() == Some("-v"))
        );
        assert!(
            flags
                .iter()
                .any(|candidate| candidate.short.as_deref() == Some("-v") && candidate.takes_value)
        );

        let subcommands = extract_subcommands_from_mdoc(&doc);
        assert!(subcommands.iter().any(|candidate| candidate.name == "sync"));
    }

    #[test]
    fn test_parse_mdoc_it_list_optional_arg_from_op() {
        let tokens = vec![
            Token::Macro {
                name: "Sh".to_string(),
                args: vec!["SYNOPSIS".to_string()],
                line: 0,
            },
            Token::Macro {
                name: "Bl".to_string(),
                args: vec!["-tag".to_string()],
                line: 1,
            },
            Token::Macro {
                name: "It".to_string(),
                args: vec!["Op".to_string(), "Ar".to_string(), "path".to_string()],
                line: 2,
            },
            Token::Macro {
                name: "El".to_string(),
                args: vec![],
                line: 3,
            },
        ];

        let doc = parse_mdoc_source(&tokens);
        let args = extract_args_from_mdoc(&doc);
        assert!(
            args.iter()
                .any(|candidate| { candidate.name == "path" && !candidate.required })
        );
    }
}
