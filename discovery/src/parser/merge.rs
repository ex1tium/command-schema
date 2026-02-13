//! Candidate merging and deterministic schema finalization.

use std::collections::HashMap;

use command_schema_core::{ArgSchema, CommandSchema, FlagSchema, SubcommandSchema};

use super::ast::{ArgCandidate, FlagCandidate, SubcommandCandidate};
use super::confidence::{score_arg_candidate, score_flag_candidate, score_subcommand_candidate};

pub use super::confidence::{HIGH_CONFIDENCE_THRESHOLD, MEDIUM_CONFIDENCE_THRESHOLD};

/// Groups candidates by acceptance tier after confidence scoring.
/// `accepted` holds schema-ready items, `medium_confidence` holds borderline
/// candidates, and `discarded` holds items below the minimum threshold.
#[derive(Debug, Clone)]
pub struct GateResult<T, C> {
    pub accepted: Vec<T>,
    pub medium_confidence: Vec<C>,
    pub discarded: Vec<C>,
}

fn choose_best_candidate<C, F>(candidates: Vec<C>, mut score_fn: F) -> (Option<C>, Vec<C>, Vec<C>)
where
    C: Clone,
    F: FnMut(&C) -> f64,
{
    let mut best: Option<C> = None;
    let mut best_score = -1.0;
    let mut medium = Vec::new();
    let mut discarded = Vec::new();

    for candidate in candidates {
        let score = score_fn(&candidate);
        if score >= HIGH_CONFIDENCE_THRESHOLD {
            if score > best_score {
                if let Some(prev) = best.take() {
                    medium.push(prev);
                }
                best = Some(candidate);
                best_score = score;
            } else {
                medium.push(candidate);
            }
        } else if score >= MEDIUM_CONFIDENCE_THRESHOLD {
            medium.push(candidate);
        } else {
            discarded.push(candidate);
        }
    }

    (best, medium, discarded)
}

/// Merges grouped `FlagCandidate`s by canonical key, scores each group,
/// and partitions into accepted/medium/discarded tiers based on `threshold`.
pub fn merge_flag_candidates(
    candidates: Vec<FlagCandidate>,
    threshold: f64,
) -> GateResult<FlagSchema, FlagCandidate> {
    let mut grouped: HashMap<String, Vec<FlagCandidate>> = HashMap::new();
    for candidate in candidates {
        grouped
            .entry(candidate.canonical_key())
            .or_default()
            .push(candidate);
    }

    let mut accepted = Vec::new();
    let mut medium_confidence = Vec::new();
    let mut discarded = Vec::new();

    for (_key, group) in grouped {
        let (best, mut medium, mut low) = choose_best_candidate(group, score_flag_candidate);
        if let Some(best_candidate) = best {
            let score = score_flag_candidate(&best_candidate);
            if score >= threshold {
                let schema = best_candidate.into_schema();
                if is_valid_flag_schema(&schema) {
                    accepted.push(schema);
                } else {
                    // Invalid names (for example bare "-" / "--") should not
                    // fail the full schema; keep them out of accepted output.
                    low.push(FlagCandidate::from_schema(
                        schema,
                        super::ast::SourceSpan::unknown(),
                        "merge-invalid-flag",
                        0.0,
                    ));
                }
            } else if score >= MEDIUM_CONFIDENCE_THRESHOLD {
                medium.push(best_candidate);
            } else {
                low.push(best_candidate);
            }
        }

        medium_confidence.append(&mut medium);
        discarded.append(&mut low);
    }

    accepted.sort_by(|a, b| a.canonical_name().cmp(b.canonical_name()));
    GateResult {
        accepted,
        medium_confidence,
        discarded,
    }
}

fn is_valid_flag_schema(flag: &FlagSchema) -> bool {
    let short_ok = flag.short.as_deref().is_none_or(|short| {
        short.starts_with('-')
            && !short.starts_with("--")
            && short.len() >= 2
            // Body must not contain brackets, slashes, parens, or angle brackets
            && !short[1..]
                .chars()
                .any(|ch| matches!(ch, '[' | ']' | '<' | '>' | '(' | ')' | '/'))
    });
    let long_ok = flag.long.as_deref().is_none_or(|long| {
        long.starts_with("--")
            && long.len() >= 3
            // Body must start with a letter (rejects "---" and ASCII art)
            && long[2..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic())
            // Body must be alphanumeric + hyphens + dots + underscores (no brackets, etc.)
            && long[2..]
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    });
    short_ok && long_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_flag_schema_rejects_bare_dash_names() {
        assert!(!is_valid_flag_schema(&FlagSchema::boolean(Some("-"), None)));
        assert!(!is_valid_flag_schema(&FlagSchema::boolean(
            None,
            Some("--")
        )));
    }

    #[test]
    fn test_is_valid_flag_schema_accepts_well_formed_flags() {
        assert!(is_valid_flag_schema(&FlagSchema::boolean(
            Some("-v"),
            Some("--verbose")
        )));
    }

    #[test]
    fn test_is_valid_flag_schema_rejects_short_with_double_dash() {
        // Short flag should not use "--" prefix
        assert!(!is_valid_flag_schema(&FlagSchema::boolean(
            Some("--v"),
            None
        )));
    }

    #[test]
    fn test_is_valid_flag_schema_boundary_long_name() {
        // "--ab" (length 4) is valid, "--" (length 2) is not
        assert!(is_valid_flag_schema(&FlagSchema::boolean(
            None,
            Some("--ab")
        )));
        assert!(!is_valid_flag_schema(&FlagSchema::boolean(
            None,
            Some("--")
        )));
    }

    #[test]
    fn test_merge_flag_candidates_drops_invalid_long_names() {
        let candidates = vec![FlagCandidate {
            short: None,
            long: Some("-bad".to_string()),
            value_type: command_schema_core::ValueType::String,
            takes_value: false,
            description: None,
            multiple: false,
            conflicts_with: Vec::new(),
            requires: Vec::new(),
            source_span: super::super::ast::SourceSpan::single(1),
            strategy: "test",
            confidence: 1.0,
        }];

        let merged = merge_flag_candidates(candidates, HIGH_CONFIDENCE_THRESHOLD);
        assert!(merged.accepted.is_empty());
    }
}

/// Merges grouped `SubcommandCandidate`s by canonical key, scores each group,
/// and partitions into accepted/medium/discarded tiers based on `threshold`.
pub fn merge_subcommand_candidates(
    candidates: Vec<SubcommandCandidate>,
    threshold: f64,
) -> GateResult<SubcommandSchema, SubcommandCandidate> {
    let mut grouped: HashMap<String, Vec<SubcommandCandidate>> = HashMap::new();
    for candidate in candidates {
        grouped
            .entry(candidate.canonical_key())
            .or_default()
            .push(candidate);
    }

    let mut accepted = Vec::new();
    let mut medium_confidence = Vec::new();
    let mut discarded = Vec::new();

    for (_key, group) in grouped {
        let (best, mut medium, mut low) = choose_best_candidate(group, score_subcommand_candidate);
        if let Some(best_candidate) = best {
            let score = score_subcommand_candidate(&best_candidate);
            if score >= threshold {
                accepted.push(best_candidate.into_schema());
            } else if score >= MEDIUM_CONFIDENCE_THRESHOLD {
                medium.push(best_candidate);
            } else {
                low.push(best_candidate);
            }
        }

        medium_confidence.append(&mut medium);
        discarded.append(&mut low);
    }

    accepted.sort_by(|a, b| a.name.cmp(&b.name));
    GateResult {
        accepted,
        medium_confidence,
        discarded,
    }
}

/// Merges grouped `ArgCandidate`s by canonical key, scores each group,
/// and partitions into accepted/medium/discarded tiers based on `threshold`.
pub fn merge_arg_candidates(
    candidates: Vec<ArgCandidate>,
    threshold: f64,
) -> GateResult<ArgSchema, ArgCandidate> {
    let mut grouped: HashMap<String, Vec<ArgCandidate>> = HashMap::new();
    for candidate in candidates {
        grouped
            .entry(candidate.canonical_key())
            .or_default()
            .push(candidate);
    }

    let mut accepted = Vec::new();
    let mut medium_confidence = Vec::new();
    let mut discarded = Vec::new();

    for (_key, group) in grouped {
        let (best, mut medium, mut low) = choose_best_candidate(group, score_arg_candidate);
        if let Some(best_candidate) = best {
            let score = score_arg_candidate(&best_candidate);
            if score >= threshold {
                accepted.push(best_candidate.into_schema());
            } else if score >= MEDIUM_CONFIDENCE_THRESHOLD {
                medium.push(best_candidate);
            } else {
                low.push(best_candidate);
            }
        }

        medium_confidence.append(&mut medium);
        discarded.append(&mut low);
    }

    accepted.sort_by(|a, b| a.name.cmp(&b.name));
    GateResult {
        accepted,
        medium_confidence,
        discarded,
    }
}

/// Sorts subcommands, flags, positional args, and aliases within a
/// `CommandSchema` for deterministic output.
pub fn finalize_schema(mut schema: CommandSchema) -> CommandSchema {
    schema.subcommands.sort_by(|a, b| a.name.cmp(&b.name));
    schema
        .global_flags
        .sort_by(|a, b| a.canonical_name().cmp(b.canonical_name()));
    schema.positional.sort_by(|a, b| a.name.cmp(&b.name));

    for subcmd in &mut schema.subcommands {
        subcmd
            .flags
            .sort_by(|a, b| a.canonical_name().cmp(b.canonical_name()));
        subcmd.aliases.sort();
        subcmd.subcommands.sort_by(|a, b| a.name.cmp(&b.name));
    }

    schema
}
