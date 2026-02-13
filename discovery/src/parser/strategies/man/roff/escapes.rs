//! Roff escape sequence handling.

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeType {
    Bold,
    Italic,
    Roman,
    Revert,
    Previous,
    Unknown,
}

#[allow(dead_code)]
pub fn classify_escape(seq: &str) -> EscapeType {
    match seq {
        "\\B" | "\\fB" => EscapeType::Bold,
        "\\I" | "\\fI" => EscapeType::Italic,
        "\\R" | "\\fR" => EscapeType::Roman,
        "\\P" => EscapeType::Previous,
        "\\fP" => EscapeType::Revert,
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
            // Font switches and style toggles.
            'f' => {
                chars.next();
                let _ = chars.next(); // consume style selector
            }
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
}
