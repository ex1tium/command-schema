//! Shared utility functions for help text detection and validation.

/// Returns `true` if `trimmed` matches a rendered man page title banner token
/// (e.g. `GIT-REBASE(1)` or `STAT(1)`).
///
/// Extracts the first whitespace-separated token to handle full title lines
/// like `GIT-REBASE(1)  Git Manual  GIT-REBASE(1)`. Uses `rfind('(')` on
/// that token for robustness, and applies permissive character rules:
///
/// - **Name part**: ASCII alphanumeric plus `-`, `_`, `.`, `+`
/// - **Section part**: ASCII digits or alphabetic characters
pub fn looks_like_man_title_line(trimmed: &str) -> bool {
    if trimmed.is_empty() {
        return false;
    }

    let first = trimmed.split_whitespace().next().unwrap_or_default();
    if !first.ends_with(')') {
        return false;
    }
    let Some(paren_idx) = first.rfind('(') else {
        return false;
    };
    if paren_idx == 0 {
        return false;
    }
    let name = &first[..paren_idx];
    let section = &first[paren_idx + 1..first.len() - 1];
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '+'))
        && !section.is_empty()
        && section
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch.is_ascii_alphabetic())
}

/// Returns `true` when a lowercased line starts with the given macro name
/// (e.g. `".sh"`) followed by ASCII whitespace or end-of-line.
pub fn starts_with_roff_macro(line: &str, macro_name: &str) -> bool {
    if !line.starts_with(macro_name) {
        return false;
    }
    line[macro_name.len()..]
        .chars()
        .next()
        .is_none_or(|ch| ch.is_ascii_whitespace())
}

/// Returns `true` when the line looks like a roff macro invocation
/// (control char `.` or `'` followed by two alphabetic characters).
pub fn is_roff_macro_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let mut chars = trimmed.chars();
    let Some(control) = chars.next() else {
        return false;
    };
    if control != '.' && control != '\'' {
        return false;
    }
    let Some(a) = chars.next() else {
        return false;
    };
    let Some(b) = chars.next() else {
        return false;
    };
    a.is_ascii_alphabetic() && b.is_ascii_alphabetic()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_title_token() {
        assert!(looks_like_man_title_line("GIT-REBASE(1)"));
        assert!(looks_like_man_title_line("STAT(1)"));
    }

    #[test]
    fn test_full_title_line_with_manual_label() {
        assert!(looks_like_man_title_line(
            "GIT-REBASE(1)                     Git Manual                     GIT-REBASE(1)"
        ));
    }

    #[test]
    fn test_permissive_name_chars() {
        assert!(looks_like_man_title_line("G++.TOOL(1)"));
        assert!(looks_like_man_title_line("MY_CMD(3p)"));
    }

    #[test]
    fn test_rejects_empty_and_invalid() {
        assert!(!looks_like_man_title_line(""));
        assert!(!looks_like_man_title_line("no-parens"));
        assert!(!looks_like_man_title_line("()"));
        assert!(!looks_like_man_title_line("(1)"));
    }
}
