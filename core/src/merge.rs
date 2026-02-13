//! Schema merging with configurable conflict resolution.
//!
//! When the same command has schemas from multiple sources (e.g., `--help`
//! extraction and user history), [`merge_schemas`] combines them into a
//! single schema using a [`MergeStrategy`] to resolve conflicts.
//!
//! # Example
//!
//! ```
//! use command_schema_core::*;
//!
//! let mut base = CommandSchema::new("git", SchemaSource::Bootstrap);
//! base.global_flags.push(FlagSchema::boolean(Some("-v"), Some("--verbose")));
//!
//! let mut overlay = CommandSchema::new("git", SchemaSource::Learned);
//! overlay.global_flags.push(
//!     FlagSchema::with_value(Some("-m"), Some("--message"), ValueType::String),
//! );
//!
//! let merged = merge_schemas(&base, &overlay, MergeStrategy::Union);
//! assert_eq!(merged.global_flags.len(), 2);
//! ```

use std::collections::HashMap;

use crate::{ArgSchema, CommandSchema, FlagSchema, SubcommandSchema};

/// Schema merge behavior.
///
/// Controls how conflicts between a base and overlay schema are resolved.
///
/// # Examples
///
/// ```
/// use command_schema_core::*;
///
/// let mut base = CommandSchema::new("git", SchemaSource::Bootstrap);
/// base.description = Some("base desc".into());
///
/// let mut overlay = CommandSchema::new("git", SchemaSource::Learned);
/// overlay.description = Some("overlay desc".into());
///
/// let m1 = merge_schemas(&base, &overlay, MergeStrategy::PreferBase);
/// assert_eq!(m1.description.as_deref(), Some("base desc"));
///
/// let m2 = merge_schemas(&base, &overlay, MergeStrategy::PreferOverlay);
/// assert_eq!(m2.description.as_deref(), Some("overlay desc"));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    /// Keep base values when conflicts occur.
    PreferBase,
    /// Keep overlay values when conflicts occur.
    PreferOverlay,
    /// Combine both with conflict-aware unions (overlay wins for description).
    Union,
}

/// Merges two command schemas into one schema.
///
/// Flags and subcommands are deduplicated by canonical name. The `strategy`
/// determines which side wins when both schemas define the same entity.
///
/// # Examples
///
/// ```
/// use command_schema_core::*;
///
/// let mut base = CommandSchema::new("git", SchemaSource::Bootstrap);
/// base.global_flags.push(FlagSchema::boolean(Some("-v"), Some("--verbose")));
///
/// let mut overlay = CommandSchema::new("git", SchemaSource::Learned);
/// overlay.global_flags.push(
///     FlagSchema::with_value(Some("-m"), Some("--message"), ValueType::String),
/// );
/// // overlay also has --verbose
/// overlay.global_flags.push(FlagSchema::boolean(Some("-v"), Some("--verbose")));
///
/// let merged = merge_schemas(&base, &overlay, MergeStrategy::Union);
/// assert_eq!(merged.global_flags.len(), 2); // deduplicated
/// ```
pub fn merge_schemas(
    base: &CommandSchema,
    overlay: &CommandSchema,
    strategy: MergeStrategy,
) -> CommandSchema {
    let mut merged = base.clone();

    merged.description = match strategy {
        MergeStrategy::PreferBase => base
            .description
            .clone()
            .or_else(|| overlay.description.clone()),
        MergeStrategy::PreferOverlay => overlay
            .description
            .clone()
            .or_else(|| base.description.clone()),
        MergeStrategy::Union => overlay
            .description
            .clone()
            .or_else(|| base.description.clone()),
    };

    merged.global_flags = merge_flags(&base.global_flags, &overlay.global_flags, strategy);
    merged.subcommands = merge_subcommands(&base.subcommands, &overlay.subcommands, strategy);
    merged.positional = merge_positional_args(&base.positional, &overlay.positional, strategy);

    // Confidence: take the max of both sources.
    merged.confidence = base.confidence.max(overlay.confidence);

    // Version: prefer overlay (usually more specific / current).
    if overlay.version.is_some() {
        merged.version = overlay.version.clone();
    }

    merged
}

fn merge_flags(
    base: &[FlagSchema],
    overlay: &[FlagSchema],
    strategy: MergeStrategy,
) -> Vec<FlagSchema> {
    // Use a canonical-key map, but also track short→canonical and long→canonical
    // so that flags with overlapping names are properly deduped even when one
    // source has a long name and the other does not.
    let mut by_canonical: HashMap<String, FlagSchema> = HashMap::new();
    let mut short_to_canonical: HashMap<String, String> = HashMap::new();
    let mut long_to_canonical: HashMap<String, String> = HashMap::new();

    let canonical_key = |flag: &FlagSchema| -> String {
        flag.long
            .clone()
            .or_else(|| flag.short.clone())
            .unwrap_or_else(|| "<unknown>".to_string())
    };

    let insert_flag =
        |flag: &FlagSchema,
         by_canonical: &mut HashMap<String, FlagSchema>,
         short_map: &mut HashMap<String, String>,
         long_map: &mut HashMap<String, String>,
         overwrite: bool| {
            // Check both names independently — they may point to different canonical keys.
            let long_key = flag.long.as_ref().and_then(|l| long_map.get(l).cloned());
            let short_key = flag.short.as_ref().and_then(|s| short_map.get(s).cloned());

            let key = match (&long_key, &short_key) {
                (Some(lk), Some(sk)) if lk != sk => {
                    // Two previously separate entries (e.g. long-only "--no-pager"
                    // and short-only "-P") are now revealed to be the same flag.
                    // Consolidate: absorb the short-keyed entry into the long-keyed one.
                    if let Some(old) = by_canonical.remove(sk) {
                        if let Some(existing) = by_canonical.get_mut(lk) {
                            if existing.short.is_none() {
                                existing.short = old.short.clone();
                            }
                            if existing.description.is_none() {
                                existing.description = old.description.clone();
                            }
                        }
                        // Remap the short name to the surviving canonical key.
                        if let Some(s) = &old.short {
                            short_map.insert(s.clone(), lk.clone());
                        }
                    }
                    lk.clone()
                }
                (Some(lk), _) => lk.clone(),
                (_, Some(sk)) => sk.clone(),
                (None, None) => canonical_key(flag),
            };

            if overwrite {
                // Merge names: if existing has a long but new doesn't (or vice versa),
                // keep the more complete version.
                if let Some(existing) = by_canonical.get(&key) {
                    let mut merged_flag = flag.clone();
                    if merged_flag.long.is_none() {
                        merged_flag.long = existing.long.clone();
                    }
                    if merged_flag.short.is_none() {
                        merged_flag.short = existing.short.clone();
                    }
                    if merged_flag.description.is_none() {
                        merged_flag.description = existing.description.clone();
                    }
                    by_canonical.insert(key.clone(), merged_flag);
                } else {
                    by_canonical.insert(key.clone(), flag.clone());
                }
            } else {
                by_canonical.entry(key.clone()).or_insert_with(|| flag.clone());
            }

            // Register all name variants for this canonical key.
            if let Some(short) = &flag.short {
                short_map.insert(short.clone(), key.clone());
            }
            if let Some(long) = &flag.long {
                long_map.insert(long.clone(), key);
            }
        };

    match strategy {
        MergeStrategy::PreferBase => {
            for flag in base {
                insert_flag(
                    flag,
                    &mut by_canonical,
                    &mut short_to_canonical,
                    &mut long_to_canonical,
                    true,
                );
            }
            for flag in overlay {
                insert_flag(
                    flag,
                    &mut by_canonical,
                    &mut short_to_canonical,
                    &mut long_to_canonical,
                    false, // don't overwrite base
                );
            }
        }
        MergeStrategy::PreferOverlay | MergeStrategy::Union => {
            for flag in base {
                insert_flag(
                    flag,
                    &mut by_canonical,
                    &mut short_to_canonical,
                    &mut long_to_canonical,
                    true,
                );
            }
            for flag in overlay {
                insert_flag(
                    flag,
                    &mut by_canonical,
                    &mut short_to_canonical,
                    &mut long_to_canonical,
                    true, // overlay wins
                );
            }
        }
    }

    let mut flags: Vec<_> = by_canonical.into_values().collect();
    flags.sort_by(|a, b| {
        let key_a = a.long.as_ref().or(a.short.as_ref());
        let key_b = b.long.as_ref().or(b.short.as_ref());
        key_a.cmp(&key_b)
    });
    flags
}

fn merge_subcommands(
    base: &[SubcommandSchema],
    overlay: &[SubcommandSchema],
    strategy: MergeStrategy,
) -> Vec<SubcommandSchema> {
    let mut map: HashMap<String, SubcommandSchema> = HashMap::new();

    for sub in base {
        map.insert(sub.name.clone(), sub.clone());
    }

    for sub in overlay {
        match map.get_mut(&sub.name) {
            Some(existing) => {
                *existing = merge_subcommand(existing, sub, strategy);
            }
            None => {
                map.insert(sub.name.clone(), sub.clone());
            }
        }
    }

    let mut subcommands: Vec<_> = map.into_values().collect();
    subcommands.sort_by(|a, b| a.name.cmp(&b.name));
    subcommands
}

fn merge_subcommand(
    base: &SubcommandSchema,
    overlay: &SubcommandSchema,
    strategy: MergeStrategy,
) -> SubcommandSchema {
    let mut merged = base.clone();
    merged.description = match strategy {
        MergeStrategy::PreferBase => base
            .description
            .clone()
            .or_else(|| overlay.description.clone()),
        MergeStrategy::PreferOverlay => overlay
            .description
            .clone()
            .or_else(|| base.description.clone()),
        MergeStrategy::Union => overlay
            .description
            .clone()
            .or_else(|| base.description.clone()),
    };

    merged.flags = merge_flags(&base.flags, &overlay.flags, strategy);
    merged.subcommands = merge_subcommands(&base.subcommands, &overlay.subcommands, strategy);

    if strategy == MergeStrategy::PreferOverlay {
        merged.positional = overlay.positional.clone();
        merged.aliases = overlay.aliases.clone();
    } else {
        if merged.positional.is_empty() {
            merged.positional = overlay.positional.clone();
        }
        if merged.aliases.is_empty() {
            merged.aliases = overlay.aliases.clone();
        }
    }

    merged
}

fn merge_positional_args(
    base: &[ArgSchema],
    overlay: &[ArgSchema],
    strategy: MergeStrategy,
) -> Vec<ArgSchema> {
    match strategy {
        MergeStrategy::PreferBase => {
            if base.is_empty() {
                overlay.to_vec()
            } else {
                base.to_vec()
            }
        }
        MergeStrategy::PreferOverlay => {
            if overlay.is_empty() {
                base.to_vec()
            } else {
                overlay.to_vec()
            }
        }
        MergeStrategy::Union => {
            // Take the longer list, fill missing descriptions from the shorter.
            let (primary, secondary) = if overlay.len() >= base.len() {
                (overlay, base)
            } else {
                (base, overlay)
            };
            let mut result = primary.to_vec();
            for (i, arg) in result.iter_mut().enumerate() {
                if arg.description.is_none() {
                    if let Some(other) = secondary.get(i) {
                        arg.description = other.description.clone();
                    }
                }
            }
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{SchemaSource, ValueType};

    use super::*;

    #[test]
    fn test_merge_prefer_base_keeps_base_description() {
        let mut base = CommandSchema::new("git", SchemaSource::Bootstrap);
        base.description = Some("base".to_string());
        let mut overlay = CommandSchema::new("git", SchemaSource::Learned);
        overlay.description = Some("overlay".to_string());

        let merged = merge_schemas(&base, &overlay, MergeStrategy::PreferBase);
        assert_eq!(merged.description.as_deref(), Some("base"));
    }

    #[test]
    fn test_merge_prefer_overlay_replaces_description() {
        let mut base = CommandSchema::new("git", SchemaSource::Bootstrap);
        base.description = Some("base".to_string());
        let mut overlay = CommandSchema::new("git", SchemaSource::Learned);
        overlay.description = Some("overlay".to_string());

        let merged = merge_schemas(&base, &overlay, MergeStrategy::PreferOverlay);
        assert_eq!(merged.description.as_deref(), Some("overlay"));
    }

    #[test]
    fn test_merge_union_deduplicates_flags() {
        let mut base = CommandSchema::new("git", SchemaSource::Bootstrap);
        base.global_flags
            .push(FlagSchema::boolean(Some("-v"), Some("--verbose")));

        let mut overlay = CommandSchema::new("git", SchemaSource::Learned);
        overlay.global_flags.push(FlagSchema::with_value(
            Some("-m"),
            Some("--message"),
            ValueType::String,
        ));
        overlay
            .global_flags
            .push(FlagSchema::boolean(Some("-v"), Some("--verbose")));

        let merged = merge_schemas(&base, &overlay, MergeStrategy::Union);
        assert_eq!(merged.global_flags.len(), 2);
    }

    #[test]
    fn test_merge_union_positional_args_takes_longer_list() {
        let mut base = CommandSchema::new("cmd", SchemaSource::HelpCommand);
        base.positional
            .push(ArgSchema::required("file", ValueType::File));

        let mut overlay = CommandSchema::new("cmd", SchemaSource::ManPage);
        overlay
            .positional
            .push(ArgSchema::required("file", ValueType::File));
        overlay
            .positional
            .push(ArgSchema::optional("extra", ValueType::String));

        let merged = merge_schemas(&base, &overlay, MergeStrategy::Union);
        assert_eq!(merged.positional.len(), 2);
    }

    #[test]
    fn test_merge_union_positional_fills_descriptions() {
        let mut base = CommandSchema::new("cmd", SchemaSource::HelpCommand);
        let mut arg = ArgSchema::required("file", ValueType::File);
        arg.description = Some("The input file".to_string());
        base.positional.push(arg);

        let mut overlay = CommandSchema::new("cmd", SchemaSource::ManPage);
        overlay
            .positional
            .push(ArgSchema::required("file", ValueType::File));
        overlay
            .positional
            .push(ArgSchema::optional("extra", ValueType::String));

        // base has description for first arg, overlay does not
        let merged = merge_schemas(&base, &overlay, MergeStrategy::Union);
        assert_eq!(merged.positional.len(), 2);
        // overlay is longer so it's primary; base's description fills in
        assert_eq!(
            merged.positional[0].description.as_deref(),
            Some("The input file")
        );
    }

    #[test]
    fn test_merge_takes_max_confidence() {
        let mut base = CommandSchema::new("cmd", SchemaSource::HelpCommand);
        base.confidence = 0.65;
        let mut overlay = CommandSchema::new("cmd", SchemaSource::ManPage);
        overlay.confidence = 0.85;

        let merged = merge_schemas(&base, &overlay, MergeStrategy::Union);
        assert!((merged.confidence - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn test_merge_prefers_overlay_version() {
        let mut base = CommandSchema::new("cmd", SchemaSource::HelpCommand);
        base.version = Some("1.0".to_string());
        let mut overlay = CommandSchema::new("cmd", SchemaSource::ManPage);
        overlay.version = Some("2.0".to_string());

        let merged = merge_schemas(&base, &overlay, MergeStrategy::Union);
        assert_eq!(merged.version.as_deref(), Some("2.0"));
    }

    #[test]
    fn test_merge_keeps_base_version_when_overlay_has_none() {
        let mut base = CommandSchema::new("cmd", SchemaSource::HelpCommand);
        base.version = Some("1.0".to_string());
        let overlay = CommandSchema::new("cmd", SchemaSource::ManPage);

        let merged = merge_schemas(&base, &overlay, MergeStrategy::Union);
        assert_eq!(merged.version.as_deref(), Some("1.0"));
    }

    #[test]
    fn test_merge_union_deduplicates_flags_by_short_name_overlap() {
        // Scenario: man page produces "-P" with long "--no-pager",
        // help produces "-P" without a long name. These should merge
        // into a single flag with both names.
        let mut base = CommandSchema::new("git", SchemaSource::HelpCommand);
        base.global_flags
            .push(FlagSchema::boolean(Some("-P"), None));

        let mut overlay = CommandSchema::new("git", SchemaSource::ManPage);
        let mut flag = FlagSchema::boolean(Some("-P"), Some("--no-pager"));
        flag.description = Some("Do not pipe output into a pager".to_string());
        overlay.global_flags.push(flag);

        let merged = merge_schemas(&base, &overlay, MergeStrategy::Union);
        assert_eq!(
            merged.global_flags.len(),
            1,
            "flags with overlapping short name must merge"
        );
        let merged_flag = &merged.global_flags[0];
        assert_eq!(merged_flag.short.as_deref(), Some("-P"));
        assert_eq!(merged_flag.long.as_deref(), Some("--no-pager"));
        assert!(merged_flag.description.is_some());
    }

    #[test]
    fn test_merge_consolidates_split_short_and_long_entries() {
        // Real-world scenario: help output produces TWO separate entries for
        // the same flag — {short: None, long: "--no-pager"} and
        // {short: "-P", long: None}. Man page has the unified entry
        // {short: "-P", long: "--no-pager"}. After merge all three must
        // collapse into a single flag.
        let mut base = CommandSchema::new("git", SchemaSource::HelpCommand);
        base.global_flags
            .push(FlagSchema::boolean(None, Some("--no-pager")));
        base.global_flags
            .push(FlagSchema::boolean(Some("-P"), None));

        let mut overlay = CommandSchema::new("git", SchemaSource::ManPage);
        let mut flag = FlagSchema::boolean(Some("-P"), Some("--no-pager"));
        flag.description = Some("Do not pipe output into a pager".to_string());
        overlay.global_flags.push(flag);

        let merged = merge_schemas(&base, &overlay, MergeStrategy::Union);

        // Must consolidate to exactly one flag with both names.
        let pager_flags: Vec<_> = merged
            .global_flags
            .iter()
            .filter(|f| {
                f.short.as_deref() == Some("-P") || f.long.as_deref() == Some("--no-pager")
            })
            .collect();
        assert_eq!(
            pager_flags.len(),
            1,
            "split short-only and long-only entries must consolidate into one flag, got: {:?}",
            pager_flags
        );
        let f = pager_flags[0];
        assert_eq!(f.short.as_deref(), Some("-P"));
        assert_eq!(f.long.as_deref(), Some("--no-pager"));
        assert_eq!(
            f.description.as_deref(),
            Some("Do not pipe output into a pager")
        );
    }
}
