//! Intermediate parser AST candidates carrying evidence metadata.

use command_schema_core::{ArgSchema, FlagSchema, SubcommandSchema, ValueType};

/// Source line range within the normalized help output, used for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    pub line_start: usize,
    pub line_end: usize,
}

impl SourceSpan {
    pub const fn single(line: usize) -> Self {
        Self {
            line_start: line,
            line_end: line,
        }
    }

    pub const fn unknown() -> Self {
        Self {
            line_start: usize::MAX,
            line_end: usize::MAX,
        }
    }

    pub const fn is_unknown(&self) -> bool {
        self.line_start == usize::MAX && self.line_end == usize::MAX
    }
}

/// A detected usage text with provenance metadata (source span, strategy, confidence).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct UsageNode {
    pub text: String,
    pub source_span: SourceSpan,
    pub strategy: &'static str,
    pub confidence: f64,
}

/// Candidate flag extracted by a parser strategy, carrying evidence metadata
/// (source span, originating strategy, confidence score) for downstream merging.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FlagCandidate {
    pub short: Option<String>,
    pub long: Option<String>,
    pub value_type: ValueType,
    pub takes_value: bool,
    pub description: Option<String>,
    pub multiple: bool,
    pub conflicts_with: Vec<String>,
    pub requires: Vec<String>,
    pub source_span: SourceSpan,
    pub strategy: &'static str,
    pub confidence: f64,
}

/// Candidate subcommand with provenance, produced by parser strategies for
/// confidence scoring and deduplication in the merge pipeline.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SubcommandCandidate {
    pub name: String,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    pub source_span: SourceSpan,
    pub strategy: &'static str,
    pub confidence: f64,
}

/// Candidate positional argument with provenance metadata for the merge pipeline.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ArgCandidate {
    pub name: String,
    pub value_type: ValueType,
    pub required: bool,
    pub multiple: bool,
    pub description: Option<String>,
    pub source_span: SourceSpan,
    pub strategy: &'static str,
    pub confidence: f64,
}

/// Candidate flag constraint (requires/conflicts_with) extracted from flag
/// descriptions, carrying provenance for confidence-based filtering.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ConstraintCandidate {
    pub flag_name: String,
    pub requires: Vec<String>,
    pub conflicts_with: Vec<String>,
    pub source_span: SourceSpan,
    pub strategy: &'static str,
    pub confidence: f64,
}

impl FlagCandidate {
    pub fn from_schema(
        flag: FlagSchema,
        source_span: SourceSpan,
        strategy: &'static str,
        confidence: f64,
    ) -> Self {
        Self {
            short: flag.short,
            long: flag.long,
            value_type: flag.value_type,
            takes_value: flag.takes_value,
            description: flag.description,
            multiple: flag.multiple,
            conflicts_with: flag.conflicts_with,
            requires: flag.requires,
            source_span,
            strategy,
            confidence,
        }
    }

    pub fn canonical_key(&self) -> String {
        self.long
            .clone()
            .or_else(|| self.short.clone())
            .unwrap_or_else(|| "unknown".to_string())
    }

    pub fn into_schema(self) -> FlagSchema {
        FlagSchema {
            short: self.short,
            long: self.long,
            value_type: self.value_type,
            takes_value: self.takes_value,
            description: self.description,
            multiple: self.multiple,
            conflicts_with: self.conflicts_with,
            requires: self.requires,
        }
    }
}

impl SubcommandCandidate {
    pub fn from_schema(
        subcommand: SubcommandSchema,
        source_span: SourceSpan,
        strategy: &'static str,
        confidence: f64,
    ) -> Self {
        Self {
            name: subcommand.name,
            description: subcommand.description,
            aliases: subcommand.aliases,
            source_span,
            strategy,
            confidence,
        }
    }

    pub fn canonical_key(&self) -> String {
        self.name.to_ascii_lowercase()
    }

    pub fn into_schema(self) -> SubcommandSchema {
        SubcommandSchema {
            name: self.name,
            description: self.description,
            flags: Vec::new(),
            positional: Vec::new(),
            subcommands: Vec::new(),
            aliases: self.aliases,
        }
    }
}

impl ArgCandidate {
    pub fn from_schema(
        arg: ArgSchema,
        source_span: SourceSpan,
        strategy: &'static str,
        confidence: f64,
    ) -> Self {
        Self {
            name: arg.name,
            value_type: arg.value_type,
            required: arg.required,
            multiple: arg.multiple,
            description: arg.description,
            source_span,
            strategy,
            confidence,
        }
    }

    pub fn canonical_key(&self) -> String {
        self.name.to_ascii_lowercase()
    }

    pub fn into_schema(self) -> ArgSchema {
        ArgSchema {
            name: self.name,
            value_type: self.value_type,
            required: self.required,
            multiple: self.multiple,
            description: self.description,
        }
    }
}
