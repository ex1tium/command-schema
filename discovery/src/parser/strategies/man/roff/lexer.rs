//! Tokenizer for raw roff source.

use super::escapes::decode_roff_escapes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Macro {
        name: String,
        args: Vec<String>,
        line: usize,
    },
    Text {
        value: String,
        line: usize,
    },
    Newline {
        line: usize,
    },
}

pub struct RoffLexer;

impl RoffLexer {
    pub fn tokenize(lines: &[&str]) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();

        for (line_idx, raw) in lines.iter().enumerate() {
            let trimmed = raw.trim_end_matches('\n');
            if trimmed.trim().is_empty() {
                tokens.push(Token::Newline { line: line_idx });
                continue;
            }

            if is_roff_comment(trimmed) {
                tokens.push(Token::Newline { line: line_idx });
                continue;
            }

            if let Some((name, args)) = parse_macro_line(trimmed) {
                tokens.push(Token::Macro {
                    name,
                    args,
                    line: line_idx,
                });
            } else {
                tokens.push(Token::Text {
                    value: decode_roff_escapes(trimmed).trim().to_string(),
                    line: line_idx,
                });
            }
        }

        Ok(tokens)
    }
}

/// Returns `true` when the line is a roff comment (`.\"` or `'\"` control sequence).
fn is_roff_comment(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.len() < 2 {
        return false;
    }
    let first = trimmed.as_bytes()[0];
    let second = trimmed.as_bytes()[1];
    (first == b'.' || first == b'\'') && second == b'"'
}

fn parse_macro_line(line: &str) -> Option<(String, Vec<String>)> {
    let trimmed = line.trim_start();
    let mut chars = trimmed.chars();
    let control = chars.next()?;
    if control != '.' && control != '\'' {
        return None;
    }

    let rest = chars.as_str().trim_start();
    if rest.is_empty() {
        return None;
    }

    let mut macro_chars = rest.chars();
    let a = macro_chars.next()?;
    if !a.is_ascii_alphabetic() {
        return None;
    }
    if let Some(b) = macro_chars.next()
        && !b.is_ascii_alphanumeric()
        && !b.is_ascii_whitespace()
    {
        return None;
    }

    let macro_name = rest
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();

    let args_start = macro_name.len();
    let args_raw = rest.get(args_start..).unwrap_or_default();
    let args = parse_macro_args(args_raw);

    Some((macro_name, args))
}

pub fn parse_macro_args(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = input.trim().chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
            }
            '\\' => {
                let Some(next) = chars.next() else {
                    break;
                };
                match next {
                    ' ' => current.push(' '),
                    '"' => current.push('"'),
                    _ => {
                        current.push('\\');
                        current.push(next);
                    }
                }
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    out.push(decode_roff_escapes(&current));
                    current.clear();
                }
            }
            other => current.push(other),
        }
    }

    if !current.is_empty() {
        out.push(decode_roff_escapes(&current));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_macro_args_handles_quotes() {
        let args = parse_macro_args("--flag \"value with spaces\" ARG");
        assert_eq!(args, vec!["--flag", "value with spaces", "ARG"]);
    }

    #[test]
    fn test_tokenize_detects_macros() {
        let lines = [".TH TEST 1", ".SH NAME", "text"];
        let tokens = RoffLexer::tokenize(&lines).expect("tokenize");
        assert!(matches!(tokens[0], Token::Macro { .. }));
        assert!(matches!(tokens[2], Token::Text { .. }));
    }
}
