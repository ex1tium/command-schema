//! Normalization for rendered man-page output.

use crate::parser::IndexedLine;

use super::sections::looks_like_section_header;

pub fn normalize_rendered_lines(lines: &[IndexedLine]) -> Vec<IndexedLine> {
    let mut out: Vec<IndexedLine> = Vec::new();

    for line in lines {
        let trimmed_end = line.text.trim_end();
        let trimmed = trimmed_end.trim();

        if should_drop_running_header_footer(trimmed) {
            continue;
        }

        if trimmed.is_empty() {
            out.push(IndexedLine {
                index: line.index,
                text: String::new(),
            });
            continue;
        }

        if line.text.starts_with(' ') || line.text.starts_with('\t') {
            let continuation = trimmed;
            if let Some(prev) = out.last_mut()
                && should_join_continuation(prev.text.as_str(), continuation)
            {
                prev.text.push(' ');
                prev.text.push_str(continuation);
                continue;
            }
        }

        out.push(IndexedLine {
            index: line.index,
            text: trimmed_end.to_string(),
        });
    }

    out
}

fn should_drop_running_header_footer(trimmed: &str) -> bool {
    if trimmed.is_empty() {
        return false;
    }

    if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return true;
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.contains(" git manual ") || lower.contains("general commands manual") {
        return true;
    }

    crate::parser::strategies::man::detect::looks_like_man_title_line(trimmed)
}

fn should_join_continuation(previous: &str, continuation: &str) -> bool {
    let prev_trimmed = previous.trim();
    if prev_trimmed.is_empty() || continuation.is_empty() {
        return false;
    }
    if looks_like_section_header(continuation) {
        return false;
    }

    let prev_starts_option = prev_trimmed.starts_with('-');
    let prev_has_two_columns = prev_trimmed.contains("  ") || prev_trimmed.contains('\t');
    let continuation_starts_option = continuation.starts_with('-');

    (prev_starts_option || prev_has_two_columns) && !continuation_starts_option
}
