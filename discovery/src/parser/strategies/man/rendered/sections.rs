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

        // Unrecognized section header (e.g. "GETTING HELP", "SEE ALSO"):
        // close the current section so its content doesn't bleed past
        // the boundary, but don't start tracking a new section.
        if is_likely_section_boundary(&line.text) {
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
            current_lines.clear();
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

/// Returns `true` when `line` is recognized as a rendered man section header
/// or looks like an unrecognized section boundary (all-caps, left-margin).
pub fn looks_like_section_header(line: &str) -> bool {
    normalize_section_name(line).is_some() || is_likely_section_boundary(line)
}

/// Returns `true` when a line looks like a top-level man-page section
/// header — all uppercase, at the left margin, not too long.
///
/// This catches unrecognized section names (e.g. "GETTING HELP",
/// "SEE ALSO", "ENVIRONMENT") that `normalize_section_name` doesn't
/// know about, so they can act as section boundaries.
fn is_likely_section_boundary(line_text: &str) -> bool {
    // Must start at the left margin (no leading whitespace).
    if line_text.starts_with(' ') || line_text.starts_with('\t') {
        return false;
    }
    let trimmed = line_text.trim();
    if trimmed.is_empty() || trimmed.len() > 48 {
        return false;
    }
    // Must have at least one letter and no lowercase letters.
    trimmed.chars().any(|ch| ch.is_ascii_alphabetic())
        && !trimmed.chars().any(|ch| ch.is_ascii_lowercase())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_likely_section_boundary_recognizes_unknown_sections() {
        assert!(is_likely_section_boundary("GETTING HELP"));
        assert!(is_likely_section_boundary("SEE ALSO"));
        assert!(is_likely_section_boundary("ENVIRONMENT"));
        assert!(is_likely_section_boundary("AUTHOR"));
        assert!(is_likely_section_boundary("BUGS"));
    }

    #[test]
    fn test_is_likely_section_boundary_rejects_indented_lines() {
        assert!(!is_likely_section_boundary("   Overview"));
        assert!(!is_likely_section_boundary("   OVERVIEW"));
        assert!(!is_likely_section_boundary("\tOVERVIEW"));
    }

    #[test]
    fn test_is_likely_section_boundary_rejects_mixed_case() {
        assert!(!is_likely_section_boundary("Getting Help"));
        assert!(!is_likely_section_boundary("See Also"));
    }

    #[test]
    fn test_is_likely_section_boundary_rejects_empty_and_long() {
        assert!(!is_likely_section_boundary(""));
        assert!(!is_likely_section_boundary(
            "THIS IS AN EXTREMELY LONG LINE THAT SHOULD NOT BE TREATED AS A SECTION HEADER"
        ));
    }

    #[test]
    fn test_identify_sections_closes_at_unrecognized_boundary() {
        let lines = vec![
            IndexedLine { index: 0, text: "SYNOPSIS".to_string() },
            IndexedLine { index: 1, text: "       cmd [options] file".to_string() },
            IndexedLine { index: 2, text: "GETTING HELP".to_string() },
            IndexedLine { index: 3, text: "       lots of prose here".to_string() },
            IndexedLine { index: 4, text: "       more prose".to_string() },
            IndexedLine { index: 5, text: "DESCRIPTION".to_string() },
            IndexedLine { index: 6, text: "       A tool that does things.".to_string() },
        ];

        let sections = identify_man_sections(&lines);

        // SYNOPSIS should have 1 content line (index 1), NOT include
        // "lots of prose" (index 3) or "more prose" (index 4).
        let synopsis = sections.iter().find(|s| s.name == "SYNOPSIS").unwrap();
        assert_eq!(synopsis.lines.len(), 1);
        assert_eq!(synopsis.lines[0].index, 1);

        // DESCRIPTION should have 1 content line (index 6).
        let desc = sections.iter().find(|s| s.name == "DESCRIPTION").unwrap();
        assert_eq!(desc.lines.len(), 1);
        assert_eq!(desc.lines[0].index, 6);

        // "GETTING HELP" should NOT appear as a section.
        assert!(sections.iter().all(|s| s.name != "GETTING HELP"));
    }
}
