//! Man page format detection.

pub use crate::parser::util::looks_like_man_title_line;

/// Normalized format buckets used by the man strategy detector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManFormat {
    /// Semantic `mdoc` macro source such as `.Dt`, `.Sh`, `.Fl`, and `.Ar`.
    Mdoc,
    /// Legacy `man` macro source such as `.TH`, `.SH`, `.TP`, and `.IP`.
    Man,
    /// Rendered manual text (no raw roff macros).
    Rendered,
}

/// Returns `true` when the first 20 lines look like raw roff source.
///
/// The input slice should contain normalized output lines; at least two macro
/// lines must be detected for a positive match.
pub fn is_raw_roff(lines: &[&str]) -> bool {
    lines
        .iter()
        .take(20)
        .filter(|line| is_roff_macro_line(line))
        .count()
        >= 2
}

/// Detects whether `lines` represent `mdoc`, `man`, rendered man output, or none.
///
/// Raw roff classification inspects up to 64 lines and prefers `.Dt`/`.TH`
/// signals when both macro families are present.
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
            || line.starts_with(".ip")
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

/// Returns `true` when `lines` resemble rendered man-page text.
///
/// Detection uses title/header, section-header, and prose hints over bounded
/// windows (`take(12)` and `take(160)`).
pub fn is_rendered_man_page(lines: &[&str]) -> bool {
    if lines.is_empty() {
        return false;
    }

    let has_title_line = lines
        .iter()
        .take(12)
        .any(|line| looks_like_man_title_line(line.trim()));

    if has_title_line {
        return true;
    }

    let mut has_name = false;
    let mut has_synopsis = false;
    let mut has_structural_section = false;

    for line in lines.iter().take(200) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed
            .trim_end_matches(':')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_uppercase();
        match normalized.as_str() {
            "NAME" => has_name = true,
            "SYNOPSIS" => has_synopsis = true,
            "OPTIONS" | "COMMAND OPTIONS" | "GLOBAL OPTIONS" | "DESCRIPTION" | "COMMANDS" => {
                has_structural_section = true
            }
            _ => {}
        }
    }

    // Manual/prose hints are supportive only; core section structure is required
    // in this branch to avoid classifying generic help as rendered man.
    let _manual_hint_present = lines.iter().take(160).any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.contains("see also") || lower.contains("git manual") || lower.contains("man page")
    });

    has_name && has_synopsis && has_structural_section
}

/// Computes a confidence score for detected man input in the `0.0..=1.0` range.
///
/// Raw roff formats score from macro density in the first 20 lines; rendered
/// format scores from recognized section-header density.
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

/// Returns `true` when `line` matches a known rendered man section header.
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

    #[test]
    fn test_rendered_detection_rejects_generic_help_sections_only() {
        let lines = [
            "NAME",
            "USAGE",
            "COMMANDS",
            "OPTIONS",
            "A generic help page without man title or synopsis section",
        ];
        assert!(!is_rendered_man_page(&lines));
    }

    #[test]
    fn test_rendered_detection_accepts_without_options_section() {
        let lines = [
            "NAME",
            "  tool - example command",
            "SYNOPSIS",
            "  tool [mode]",
            "DESCRIPTION",
            "  Detailed manual-style description text.",
        ];
        assert!(is_rendered_man_page(&lines));
    }

    #[test]
    fn test_rendered_detection_accepts_uppercase_section_codes() {
        let lines = [
            "FOO(1M)                     User Commands                    FOO(1M)",
            "BAR(3P)",
            "NAME",
            "SYNOPSIS",
            "DESCRIPTION",
        ];
        assert!(is_rendered_man_page(&lines));
    }
}
