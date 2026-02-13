//! Pluggable parser strategies for different help-output structures.

pub mod gnu;
pub mod man;
pub mod npm;
pub mod section;
pub mod usage;

use super::ast::{ArgCandidate, FlagCandidate, SubcommandCandidate};
use super::{FormatScore, HelpParser, IndexedLine};

/// Pluggable strategy for extracting CLI schema candidates from help output.
///
/// Each strategy targets a different help-output structure (section-based,
/// GNU-style, npm-style, usage-line). Strategies are run in priority order
/// determined by [`ranked_strategy_names`].
pub trait ParserStrategy {
    fn name(&self) -> &'static str;
    fn collect_flags(&self, parser: &HelpParser, lines: &[IndexedLine]) -> Vec<FlagCandidate>;
    fn collect_subcommands(
        &self,
        parser: &HelpParser,
        lines: &[IndexedLine],
    ) -> Vec<SubcommandCandidate>;
    fn collect_args(&self, parser: &HelpParser, lines: &[IndexedLine]) -> Vec<ArgCandidate>;
}

/// Returns strategy names in priority order based on format classification scores.
///
/// "man" is included first only when `man_detected` is `true` (the classifier
/// scored `HelpFormat::Man` highest, or roff/rendered detection succeeded).
/// "section" follows for explicit headers, "npm" is added when the top format
/// is Cobra-style, then "gnu" and "usage" provide fallback coverage.
pub fn ranked_strategy_names(format_scores: &[FormatScore], man_detected: bool) -> Vec<&'static str> {
    let mut names = Vec::new();

    if man_detected {
        names.push("man");
    }
    names.push("section");

    if format_scores
        .first()
        .is_some_and(|score| score.format_label() == "cobra")
    {
        names.push("npm");
    }

    names.push("gnu");
    names.push("usage");
    names
}

trait FormatScoreExt {
    fn format_label(&self) -> &'static str;
}

impl FormatScoreExt for FormatScore {
    fn format_label(&self) -> &'static str {
        match self.format {
            command_schema_core::HelpFormat::Clap => "clap",
            command_schema_core::HelpFormat::Cobra => "cobra",
            command_schema_core::HelpFormat::Argparse => "argparse",
            command_schema_core::HelpFormat::Docopt => "docopt",
            command_schema_core::HelpFormat::Gnu => "gnu",
            command_schema_core::HelpFormat::Bsd => "bsd",
            command_schema_core::HelpFormat::Man => "man",
            command_schema_core::HelpFormat::Unknown => "unknown",
        }
    }
}
