//! Section identification for rendered man-page text.

use crate::parser::IndexedLine;

/// A parsed rendered-man section with source metadata and body lines.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ManSection {
    /// Normalized section name (for example `SYNOPSIS` or `OPTIONS`).
    pub name: String,
    /// Index of the header line that started this section (inclusive).
    pub start_line: usize,
    /// Index of the final content line in this section (inclusive).
    pub end_line: usize,
    /// Content lines belonging to this section (header line excluded).
    pub lines: Vec<IndexedLine>,
}

/// Splits rendered man text into recognized top-level sections.
///
/// Returns sections in input order and includes only sections with at least one
/// captured content line.
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

/// Returns `true` when `line` is recognized as a rendered man section header.
pub fn looks_like_section_header(line: &str) -> bool {
    normalize_section_name(line).is_some()
}

/// Normalizes a potential section header into its canonical uppercase name.
///
/// Returns `None` when `line` is not a known header used by the rendered parser.
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
