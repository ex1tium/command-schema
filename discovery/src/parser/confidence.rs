//! Candidate confidence scoring and schema gating.

use command_schema_core::CommandSchema;

use super::ast::{ArgCandidate, FlagCandidate, SubcommandCandidate};
use super::classify;

/// Minimum confidence score (0.7) for a candidate to be accepted outright.
pub const HIGH_CONFIDENCE_THRESHOLD: f64 = 0.7;

/// Minimum confidence score (0.5) for a candidate to be kept as medium-confidence.
/// Candidates below this threshold are discarded.
pub const MEDIUM_CONFIDENCE_THRESHOLD: f64 = 0.5;

/// Scores a `FlagCandidate`, applying bonuses for value-taking flags and
/// penalties for placeholder tokens. Returns a clamped [0.0, 1.0] score.
pub fn score_flag_candidate(candidate: &FlagCandidate) -> f64 {
    let mut score = candidate.confidence;

    if candidate.takes_value {
        score += 0.05;
    }

    if candidate
        .description
        .as_ref()
        .is_some_and(|desc| desc.contains('='))
    {
        score += 0.1;
    }

    if candidate
        .long
        .as_deref()
        .or(candidate.short.as_deref())
        .is_some_and(classify::is_placeholder_token)
    {
        score -= 0.5;
    }

    score.clamp(0.0, 1.0)
}

/// Scores a `SubcommandCandidate`, applying penalties for placeholder tokens,
/// env-var rows, and keybinding rows. Returns a clamped [0.0, 1.0] score.
pub fn score_subcommand_candidate(candidate: &SubcommandCandidate) -> f64 {
    let mut score = candidate.confidence;

    if classify::is_placeholder_token(candidate.name.as_str()) {
        score -= 0.7;
    }
    if classify::is_env_var_row(candidate.name.as_str()) {
        score -= 0.7;
    }
    if classify::is_keybinding_row(candidate.name.as_str()) {
        score -= 0.5;
    }

    score.clamp(0.0, 1.0)
}

/// Scores an `ArgCandidate`, applying a penalty for placeholder tokens.
/// Returns a clamped [0.0, 1.0] score.
pub fn score_arg_candidate(candidate: &ArgCandidate) -> f64 {
    let mut score = candidate.confidence;

    if classify::is_placeholder_token(candidate.name.as_str()) {
        score -= 0.45;
    }

    score.clamp(0.0, 1.0)
}

/// Enforces a minimum confidence floor on a `CommandSchema`.
/// If `schema.confidence` is below `MEDIUM_CONFIDENCE_THRESHOLD`, it is raised to that value.
pub fn gate_schema(mut schema: CommandSchema) -> Option<CommandSchema> {
    if schema.confidence < MEDIUM_CONFIDENCE_THRESHOLD {
        schema.confidence = MEDIUM_CONFIDENCE_THRESHOLD;
    }

    Some(schema)
}
