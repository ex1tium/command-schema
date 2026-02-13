//! Man-page strategy (raw roff + rendered man output).

pub mod detect;
pub mod rendered;
pub mod roff;

use crate::parser::ast::{ArgCandidate, FlagCandidate, SourceSpan, SubcommandCandidate};
use crate::parser::strategies::ParserStrategy;
use crate::parser::{HelpParser, IndexedLine};

#[derive(Debug, Default)]
pub struct CandidateBundle {
    pub flags: Vec<FlagCandidate>,
    pub subcommands: Vec<SubcommandCandidate>,
    pub args: Vec<ArgCandidate>,
    pub format: Option<detect::ManFormat>,
}

impl CandidateBundle {
    pub fn has_entities(&self) -> bool {
        !self.flags.is_empty() || !self.subcommands.is_empty() || !self.args.is_empty()
    }

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
        if span.line_start == 0 && span.line_end == 0 {
            continue;
        }
        out.extend(span.line_start..=span.line_end);
    }
}

pub struct ManStrategy;

impl ManStrategy {
    pub fn collect_all(&self, _parser: &HelpParser, lines: &[IndexedLine]) -> CandidateBundle {
        let refs = lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();

        let detected = detect::detect_roff_variant(&refs);

        if let Some(format @ (detect::ManFormat::Mdoc | detect::ManFormat::Man)) = detected {
            let tokens = match roff::lexer::RoffLexer::tokenize(&refs) {
                Ok(tokens) => tokens,
                Err(_) => Vec::new(),
            };
            if !tokens.is_empty() {
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
            }
        }

        if detected == Some(detect::ManFormat::Rendered) || detect::is_rendered_man_page(&refs) {
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

impl ParserStrategy for ManStrategy {
    fn name(&self) -> &'static str {
        "man"
    }

    fn collect_flags(&self, parser: &HelpParser, lines: &[IndexedLine]) -> Vec<FlagCandidate> {
        self.collect_all(parser, lines).flags
    }

    fn collect_subcommands(
        &self,
        parser: &HelpParser,
        lines: &[IndexedLine],
    ) -> Vec<SubcommandCandidate> {
        self.collect_all(parser, lines).subcommands
    }

    fn collect_args(&self, parser: &HelpParser, lines: &[IndexedLine]) -> Vec<ArgCandidate> {
        self.collect_all(parser, lines).args
    }
}
