//! Section identification for rendered man-page text.

use crate::parser::IndexedLine;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ManSection {
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub lines: Vec<IndexedLine>,
}

pub fn identify_man_sections(lines: &[IndexedLine]) -> Vec<ManSection> {
    let mut sections = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_start = 0usize;
    let mut current_lines: Vec<IndexedLine> = Vec::new();

    for line in lines {
        let trimmed = line.text.trim();
        if let Some(header) = normalize_section_name(trimmed) {
            if let Some(name) = current_name.take()
                && !current_lines.is_empty()
            {
                sections.push(ManSection {
                    name,
                    start_line: current_start,
                    end_line: current_lines.last().map_or(current_start, |l| l.index),
                    lines: std::mem::take(&mut current_lines),
                });
            }

            current_name = Some(header);
            current_start = line.index;
            continue;
        }

        if current_name.is_some() {
            current_lines.push(line.clone());
        }
    }

    if let Some(name) = current_name.take()
        && !current_lines.is_empty()
    {
        sections.push(ManSection {
            name,
            start_line: current_start,
            end_line: current_lines.last().map_or(current_start, |l| l.index),
            lines: current_lines,
        });
    }

    sections
}

pub fn looks_like_section_header(line: &str) -> bool {
    normalize_section_name(line).is_some()
}

pub fn normalize_section_name(line: &str) -> Option<String> {
    let trimmed = line.trim().trim_end_matches(':');
    if trimmed.is_empty() || trimmed.len() > 48 {
        return None;
    }

    let compact = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    let upper = compact.to_ascii_uppercase();

    match upper.as_str() {
        "NAME" | "SYNOPSIS" | "DESCRIPTION" | "OPTIONS" | "COMMAND OPTIONS" | "GLOBAL OPTIONS"
        | "COMMANDS" | "SUBCOMMANDS" | "ARGUMENTS" | "EXAMPLES" | "EXIT STATUS" => Some(upper),
        _ => None,
    }
}
