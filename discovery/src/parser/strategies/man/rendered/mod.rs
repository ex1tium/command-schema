//! Rendered man-page extraction.

pub mod commands;
pub mod normalize;
pub mod options;
pub mod sections;
pub mod synopsis;

use crate::parser::IndexedLine;
use crate::parser::ast::{ArgCandidate, FlagCandidate, SubcommandCandidate};

#[derive(Debug, Default)]
pub struct RenderedExtraction {
    pub flags: Vec<FlagCandidate>,
    pub subcommands: Vec<SubcommandCandidate>,
    pub args: Vec<ArgCandidate>,
}

pub fn parse_candidates(lines: &[IndexedLine]) -> RenderedExtraction {
    let normalized = normalize::normalize_rendered_lines(lines);
    let sections = sections::identify_man_sections(&normalized);

    let mut extraction = RenderedExtraction::default();

    for section in &sections {
        let name = section.name.as_str();
        if name.contains("OPTION") {
            extraction
                .flags
                .extend(options::parse_options_section(section));
        }
        if name.contains("SYNOPSIS") {
            extraction
                .flags
                .extend(synopsis::parse_synopsis_flags(section));
            extraction
                .args
                .extend(synopsis::parse_synopsis_args(section));
        }
        if name.contains("COMMAND") {
            extraction
                .subcommands
                .extend(commands::parse_commands_section(section));
        }
    }

    extraction
}
