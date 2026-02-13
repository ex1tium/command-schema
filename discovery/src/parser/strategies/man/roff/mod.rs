//! Raw roff (mdoc/man macro) parsing.

pub mod escapes;
pub mod lexer;
pub mod man;
pub mod mdoc;

use crate::parser::ast::{ArgCandidate, FlagCandidate, SubcommandCandidate};

use super::detect::ManFormat;

#[derive(Debug, Default)]
pub struct RoffExtraction {
    pub flags: Vec<FlagCandidate>,
    pub subcommands: Vec<SubcommandCandidate>,
    pub args: Vec<ArgCandidate>,
}

pub fn parse_candidates(format: ManFormat, tokens: &[lexer::Token]) -> RoffExtraction {
    match format {
        ManFormat::Mdoc => {
            let doc = mdoc::parse_mdoc_source(tokens);
            RoffExtraction {
                flags: mdoc::extract_flags_from_mdoc(&doc),
                subcommands: mdoc::extract_subcommands_from_mdoc(&doc),
                args: mdoc::extract_args_from_mdoc(&doc),
            }
        }
        ManFormat::Man => {
            let doc = man::parse_man_source(tokens);
            RoffExtraction {
                flags: man::extract_flags_from_man(&doc),
                subcommands: man::extract_subcommands_from_man(&doc),
                args: man::extract_args_from_man(&doc),
            }
        }
        ManFormat::Rendered => RoffExtraction::default(),
    }
}
