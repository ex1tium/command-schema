//! Man-page strategy (raw roff + rendered man output).

pub mod detect;
pub mod rendered;
pub mod roff;

use std::cell::RefCell;

use command_schema_core::ValueType;

use crate::parser::ast::{ArgCandidate, FlagCandidate, SourceSpan, SubcommandCandidate};
use crate::parser::strategies::ParserStrategy;
use crate::parser::{HelpParser, IndexedLine};

/// Infers a [`ValueType`] from a token or description string by checking for
/// common keywords (file/path, dir, url, num/count/number).
pub fn infer_value_type(text: &str) -> ValueType {
    let lower = text.to_ascii_lowercase();
    if lower.contains("file") || lower.contains("path") {
        ValueType::File
    } else if lower.contains("dir") {
        ValueType::Directory
    } else if lower.contains("url") {
        ValueType::Url
    } else if lower.contains("num") || lower.contains("count") || lower.contains("number") {
        ValueType::Number
    } else {
        ValueType::String
    }
}

/// Combined extraction output produced by [`ManStrategy`].
///
/// The vectors hold man-derived candidates; `format` indicates which man input
/// shape was detected while extracting them.
#[derive(Debug, Default, Clone)]
pub struct CandidateBundle {
    /// Flag candidates extracted from raw roff or rendered man text.
    pub flags: Vec<FlagCandidate>,
    /// Subcommand candidates extracted from man command sections or item lists.
    pub subcommands: Vec<SubcommandCandidate>,
    /// Positional argument candidates extracted from synopsis patterns.
    pub args: Vec<ArgCandidate>,
    /// Detected man format used during extraction, if any.
    pub format: Option<detect::ManFormat>,
}

impl CandidateBundle {
    /// Returns `true` when at least one flags/subcommands/args candidate exists.
    pub fn has_entities(&self) -> bool {
        !self.flags.is_empty() || !self.subcommands.is_empty() || !self.args.is_empty()
    }

    /// Collects unique source line indices recognized by candidates in this bundle.
    pub fn recognized_indices(&self) -> Vec<usize> {
        let mut out = Vec::new();
        collect_span_indices(
            self.flags.iter().map(|candidate| candidate.source_span),
            &mut out,
        );
        collect_span_indices(
            self.subcommands
                .iter()
                .map(|candidate| candidate.source_span),
            &mut out,
        );
        collect_span_indices(
            self.args.iter().map(|candidate| candidate.source_span),
            &mut out,
        );
        out.sort_unstable();
        out.dedup();
        out
    }
}

fn collect_span_indices(spans: impl Iterator<Item = SourceSpan>, out: &mut Vec<usize>) {
    for span in spans {
        if span.is_unknown() {
            continue;
        }
        out.extend(span.line_start..=span.line_end);
    }
}

/// Cache entry storing a [`CandidateBundle`] keyed by the parser/lines identity.
struct CachedBundle {
    /// Identity of the `HelpParser` reference (pointer cast to `usize`).
    parser_id: usize,
    /// Identity of the `IndexedLine` slice start (pointer cast to `usize`).
    lines_id: usize,
    /// Length of the `IndexedLine` slice.
    lines_len: usize,
    /// Cached extraction result.
    bundle: CandidateBundle,
}

/// Parser strategy that prioritizes man-page extraction before generic help parsing.
pub struct ManStrategy {
    cache: RefCell<Option<CachedBundle>>,
}

impl ManStrategy {
    /// Creates a new `ManStrategy` with an empty result cache.
    pub fn new() -> Self {
        Self {
            cache: RefCell::new(None),
        }
    }

    /// Returns a cached [`CandidateBundle`] when the caller identity matches,
    /// otherwise computes via [`collect_all`](Self::collect_all), stores the
    /// result, and returns a clone.
    fn get_or_compute_bundle(
        &self,
        parser: &HelpParser,
        lines: &[IndexedLine],
    ) -> CandidateBundle {
        let parser_id = parser as *const HelpParser as usize;
        let lines_id = lines.as_ptr() as usize;
        let lines_len = lines.len();

        {
            let cache = self.cache.borrow();
            if let Some(cached) = cache.as_ref() {
                if cached.parser_id == parser_id
                    && cached.lines_id == lines_id
                    && cached.lines_len == lines_len
                {
                    return cached.bundle.clone();
                }
            }
        }

        let bundle = self.collect_all(parser, lines);
        let result = bundle.clone();
        *self.cache.borrow_mut() = Some(CachedBundle {
            parser_id,
            lines_id,
            lines_len,
            bundle,
        });
        result
    }

    /// Extracts flags, subcommands, and args from man-oriented input lines.
    ///
    /// Raw roff (`mdoc`/`man`) is attempted first; rendered parsing is used as
    /// fallback when raw parsing is unavailable or yields no entities.
    pub fn collect_all(&self, _parser: &HelpParser, lines: &[IndexedLine]) -> CandidateBundle {
        let refs = lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();

        let detected = detect::detect_roff_variant(&refs);
        let mut roff_failed = false;

        if let Some(format @ (detect::ManFormat::Mdoc | detect::ManFormat::Man)) = detected {
            let tokens = match roff::lexer::RoffLexer::tokenize(&refs) {
                Ok(tokens) => tokens,
                Err(_) => Vec::new(),
            };
            if tokens.is_empty() {
                roff_failed = true;
            } else {
                let parsed = roff::parse_candidates(format, &tokens);
                let bundle = CandidateBundle {
                    flags: parsed.flags,
                    subcommands: parsed.subcommands,
                    args: parsed.args,
                    format: Some(format),
                };
                if bundle.has_entities() {
                    return bundle;
                }
                roff_failed = true;
            }
        }

        if roff_failed
            || detected == Some(detect::ManFormat::Rendered)
            || detect::is_rendered_man_page(&refs)
        {
            let parsed = rendered::parse_candidates(lines);
            return CandidateBundle {
                flags: parsed.flags,
                subcommands: parsed.subcommands,
                args: parsed.args,
                format: Some(detect::ManFormat::Rendered),
            };
        }

        CandidateBundle::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_span_indices_keeps_real_line_zero() {
        let bundle = CandidateBundle {
            flags: vec![FlagCandidate {
                short: Some("-v".to_string()),
                long: None,
                value_type: command_schema_core::ValueType::Bool,
                takes_value: false,
                description: None,
                multiple: false,
                conflicts_with: Vec::new(),
                requires: Vec::new(),
                source_span: SourceSpan::single(0),
                strategy: "test",
                confidence: 1.0,
            }],
            ..Default::default()
        };
        assert_eq!(bundle.recognized_indices(), vec![0]);
    }

    #[test]
    fn test_collect_span_indices_skips_unknown_spans() {
        let bundle = CandidateBundle {
            flags: vec![FlagCandidate {
                short: Some("-v".to_string()),
                long: None,
                value_type: command_schema_core::ValueType::Bool,
                takes_value: false,
                description: None,
                multiple: false,
                conflicts_with: Vec::new(),
                requires: Vec::new(),
                source_span: SourceSpan::unknown(),
                strategy: "test",
                confidence: 1.0,
            }],
            ..Default::default()
        };
        assert!(bundle.recognized_indices().is_empty());
    }
}

impl ParserStrategy for ManStrategy {
    fn name(&self) -> &'static str {
        "man"
    }

    fn collect_flags(&self, parser: &HelpParser, lines: &[IndexedLine]) -> Vec<FlagCandidate> {
        self.get_or_compute_bundle(parser, lines).flags
    }

    fn collect_subcommands(
        &self,
        parser: &HelpParser,
        lines: &[IndexedLine],
    ) -> Vec<SubcommandCandidate> {
        self.get_or_compute_bundle(parser, lines).subcommands
    }

    fn collect_args(&self, parser: &HelpParser, lines: &[IndexedLine]) -> Vec<ArgCandidate> {
        self.get_or_compute_bundle(parser, lines).args
    }
}
