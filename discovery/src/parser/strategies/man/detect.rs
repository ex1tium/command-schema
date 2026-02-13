//! Man page format detection.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManFormat {
    /// Semantic mdoc macro format (.Dt, .Sh, .Fl, .Ar ...)
    Mdoc,
    /// Legacy man macro format (.TH, .SH, .TP, .IP ...)
    Man,
    /// Already rendered manual output.
    Rendered,
}

pub fn is_raw_roff(lines: &[&str]) -> bool {
    lines
        .iter()
        .take(20)
        .filter(|line| is_roff_macro_line(line))
        .count()
        >= 2
}

pub fn detect_roff_variant(lines: &[&str]) -> Option<ManFormat> {
    if !is_raw_roff(lines) {
        if is_rendered_man_page(lines) {
            return Some(ManFormat::Rendered);
        }
        return None;
    }

    let lower = lines
        .iter()
        .take(64)
        .map(|line| line.trim_start().to_ascii_lowercase())
        .collect::<Vec<_>>();

    let has_mdoc = lower.iter().any(|line| {
        line.starts_with(".dt ")
            || line.starts_with(".dd ")
            || line.starts_with(".sh ")
            || line.starts_with(".ss ")
            || line.starts_with(".fl ")
            || line.starts_with(".ar ")
    });

    let has_man = lower.iter().any(|line| {
        line.starts_with(".th ")
            || line.starts_with(".sh ")
            || line.starts_with(".ss ")
            || line.starts_with(".tp")
            || line.starts_with(".ip ")
    });

    match (has_mdoc, has_man) {
        (true, false) => Some(ManFormat::Mdoc),
        (false, true) => Some(ManFormat::Man),
        (true, true) => {
            // Prefer the format-specific title macro when available.
            if lower.iter().any(|line| line.starts_with(".dt ")) {
                Some(ManFormat::Mdoc)
            } else if lower.iter().any(|line| line.starts_with(".th ")) {
                Some(ManFormat::Man)
            } else {
                Some(ManFormat::Man)
            }
        }
        (false, false) => None,
    }
}

pub fn is_rendered_man_page(lines: &[&str]) -> bool {
    if lines.is_empty() {
        return false;
    }

    let header_hits = lines
        .iter()
        .take(12)
        .filter(|line| looks_like_man_title_line(line.trim()))
        .count();

    let section_hits = lines
        .iter()
        .filter(|line| looks_like_rendered_section_header(line))
        .count();

    let prose_hits = lines
        .iter()
        .take(160)
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("see also") || lower.contains("git manual") || lower.contains("man page")
        })
        .count();

    header_hits > 0 || (section_hits >= 2 && prose_hits > 0) || section_hits >= 3
}

#[allow(dead_code)]
pub fn man_page_confidence(format: ManFormat, lines: &[&str]) -> f64 {
    match format {
        ManFormat::Mdoc | ManFormat::Man => {
            let macro_hits = lines
                .iter()
                .take(20)
                .filter(|line| is_roff_macro_line(line))
                .count();
            if macro_hits >= 3 { 0.95 } else { 0.90 }
        }
        ManFormat::Rendered => {
            let sections = lines
                .iter()
                .filter(|line| looks_like_rendered_section_header(line))
                .count();
            (0.70 + 0.05 * (sections.min(4) as f64)).clamp(0.0, 0.90)
        }
    }
}

pub fn looks_like_rendered_section_header(line: &str) -> bool {
    let trimmed = line.trim().trim_end_matches(':');
    if trimmed.is_empty() || trimmed.len() > 48 {
        return false;
    }

    let normalized = trimmed
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();

    matches!(
        normalized.as_str(),
        "NAME"
            | "SYNOPSIS"
            | "DESCRIPTION"
            | "OPTIONS"
            | "COMMANDS"
            | "SUBCOMMANDS"
            | "COMMAND OPTIONS"
            | "GLOBAL OPTIONS"
            | "ARGUMENTS"
            | "EXAMPLES"
            | "EXIT STATUS"
    )
}

pub fn looks_like_man_title_line(trimmed: &str) -> bool {
    if trimmed.is_empty() {
        return false;
    }

    // Example: GIT-REBASE(1)                     Git Manual                     GIT-REBASE(1)
    let first = trimmed.split_whitespace().next().unwrap_or_default();
    if !first.contains('(') || !first.ends_with(')') {
        return false;
    }
    let Some((name, section_with_paren)) = first.split_once('(') else {
        return false;
    };
    let section = section_with_paren.trim_end_matches(')');

    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
        && !section.is_empty()
        && section
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
}

fn is_roff_macro_line(line: &str) -> bool {
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
    fn test_detect_mdoc() {
        let lines = [".Dt GIT-REBASE 1", ".Dd February 2026", ".Sh NAME", ".Fl v"];
        assert_eq!(detect_roff_variant(&lines), Some(ManFormat::Mdoc));
    }

    #[test]
    fn test_detect_man() {
        let lines = [".TH GIT-REBASE 1", ".SH NAME", ".TP", ".B --continue"];
        assert_eq!(detect_roff_variant(&lines), Some(ManFormat::Man));
    }

    #[test]
    fn test_detect_rendered() {
        let lines = [
            "GIT-REBASE(1)                     Git Manual                     GIT-REBASE(1)",
            "NAME",
            "SYNOPSIS",
            "OPTIONS",
        ];
        assert_eq!(detect_roff_variant(&lines), Some(ManFormat::Rendered));
    }
}
