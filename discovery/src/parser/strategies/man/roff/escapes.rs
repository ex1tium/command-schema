//! Roff escape sequence handling.

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeType {
    Bold,
    Italic,
    Roman,
    Revert,
    Unknown,
}

/// Classifies a roff escape sequence into an [`EscapeType`].
///
/// Recognizes both standard `\f`-prefixed font escapes (`\fB`, `\fI`, `\fR`,
/// `\fP`) and their non-standard shorthand forms (`\B`, `\I`, `\R`, `\P`)
/// which appear in some third-party and legacy man page sources.
#[allow(dead_code)]
pub fn classify_escape(seq: &str) -> EscapeType {
    match seq {
        "\\B" | "\\fB" => EscapeType::Bold,
        "\\I" | "\\fI" => EscapeType::Italic,
        "\\R" | "\\fR" => EscapeType::Roman,
        "\\P" | "\\fP" => EscapeType::Revert,
        _ => EscapeType::Unknown,
    }
}

pub fn decode_roff_escapes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        let Some(next) = chars.peek().copied() else {
            out.push(ch);
            break;
        };

        match next {
            // Font switches and style toggles (standard `\f` prefix).
            'f' => {
                chars.next(); // consume 'f'
                // Consume selector: \f[name] (bracketed), \f(XY (two-char), or \fX (single-char)
                match chars.peek().copied() {
                    Some('[') => {
                        chars.next();
                        while let Some(&ch) = chars.peek() {
                            chars.next();
                            if ch == ']' {
                                break;
                            }
                        }
                    }
                    Some('(') => {
                        // \f(XY: two-character font name (no closing paren)
                        chars.next(); // consume '('
                        let _ = chars.next(); // first char
                        let _ = chars.next(); // second char
                    }
                    Some(_) => {
                        let _ = chars.next(); // consume single style selector
                    }
                    None => {}
                }
            }
            // Non-standard shorthand font escapes (`\B`, `\I`, `\R`, `\P`)
            // found in some third-party and legacy man page sources.
            'B' | 'I' | 'R' | 'P' => {
                chars.next();
            }
            // Escaped space.
            ' ' => {
                chars.next();
                out.push(' ');
            }
            // Escaped punctuation.
            '\\' => {
                chars.next();
                out.push('\\');
            }
            '-' => {
                chars.next();
                out.push('-');
            }
            '&' => {
                chars.next();
            }
            // Special character escapes: \(XX (two-char name like \(em, \(en).
            '(' => {
                chars.next(); // consume '('
                let _ = chars.next(); // first char of name
                let _ = chars.next(); // second char of name
            }
            // Keep unknown escape payload as plain text when possible.
            other => {
                chars.next();
                out.push(other);
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_roff_escapes_basic() {
        assert_eq!(decode_roff_escapes("\\fB--help\\fR"), "--help");
        assert_eq!(
            decode_roff_escapes("path\\ with\\ space"),
            "path with space"
        );
    }

    #[test]
    fn test_decode_roff_escapes_bracketed_font_selectors() {
        // \f[BI] — bracketed font name
        assert_eq!(decode_roff_escapes("\\f[BI]foo\\fP"), "foo");
        // \f(CR — two-character font name (no closing paren)
        assert_eq!(decode_roff_escapes("\\f(CRbar\\fR"), "bar");
        // \f[B] and \f[R] — single-char names in brackets
        assert_eq!(
            decode_roff_escapes("\\f[B]--verbose\\f[R]"),
            "--verbose"
        );
    }
}
