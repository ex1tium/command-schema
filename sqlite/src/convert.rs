//! Bidirectional conversion between [`CommandSchema`] and SQLite rows.
//!
//! Handles inserting command schemas into the normalized table structure and
//! reconstructing them from SQL queries. Preserves full round-trip fidelity
//! including choices, aliases, and flag relationships.
//!
//! # Round-trip guarantees
//!
//! All fields of [`CommandSchema`] are preserved through insert → load cycles:
//! - `ValueType::Choice` variants store choices in separate tables
//! - Flag relationships (`conflicts_with`, `requires`) are stored as joins
//! - Subcommand aliases are stored in a dedicated table
//! - Nested subcommands are handled recursively with parent tracking
//!
//! # Internal API
//!
//! Most functions in this module are `pub(crate)` and used by
//! [`Migration`](crate::Migration) and [`SchemaQuery`](crate::SchemaQuery).
//! The public functions ([`insert_command`], [`insert_flags`], etc.) enable
//! direct table-level operations when needed.

use std::collections::HashMap;

use command_schema_core::{
    ArgSchema, CommandSchema, FlagSchema, SchemaSource, SubcommandSchema, ValueType,
};
use rusqlite::{Connection, params};

use crate::error::{Result, SqliteError};

/// Converts a [`ValueType`] to its string representation for storage.
///
/// [`ValueType::Choice`] variants are stored as JSON arrays; all others
/// use simple string names.
pub(crate) fn value_type_to_string(vt: &ValueType) -> String {
    match vt {
        ValueType::Bool => "Bool".to_string(),
        ValueType::String => "String".to_string(),
        ValueType::Number => "Number".to_string(),
        ValueType::File => "File".to_string(),
        ValueType::Directory => "Directory".to_string(),
        ValueType::Url => "Url".to_string(),
        ValueType::Branch => "Branch".to_string(),
        ValueType::Remote => "Remote".to_string(),
        ValueType::Choice(_) => "Choice".to_string(),
        ValueType::Any => "Any".to_string(),
    }
}

/// Parses a stored string back into a [`ValueType`].
///
/// The `Choice` variant is reconstructed by loading choices from the
/// appropriate choices table separately.
pub(crate) fn string_to_value_type(s: &str, choices: Vec<String>) -> Result<ValueType> {
    match s {
        "Bool" => Ok(ValueType::Bool),
        "String" => Ok(ValueType::String),
        "Number" => Ok(ValueType::Number),
        "File" => Ok(ValueType::File),
        "Directory" => Ok(ValueType::Directory),
        "Url" => Ok(ValueType::Url),
        "Branch" => Ok(ValueType::Branch),
        "Remote" => Ok(ValueType::Remote),
        "Choice" => Ok(ValueType::Choice(choices)),
        "Any" => Ok(ValueType::Any),
        other => Err(SqliteError::ConversionError(format!(
            "unknown value type: {other}"
        ))),
    }
}

/// Converts a [`SchemaSource`] to its string representation.
pub(crate) fn source_to_string(source: &SchemaSource) -> &'static str {
    match source {
        SchemaSource::HelpCommand => "HelpCommand",
        SchemaSource::ManPage => "ManPage",
        SchemaSource::Bootstrap => "Bootstrap",
        SchemaSource::Learned => "Learned",
    }
}

/// Parses a stored string back into a [`SchemaSource`].
pub(crate) fn string_to_source(s: &str) -> Result<SchemaSource> {
    match s {
        "HelpCommand" => Ok(SchemaSource::HelpCommand),
        "ManPage" => Ok(SchemaSource::ManPage),
        "Bootstrap" => Ok(SchemaSource::Bootstrap),
        "Learned" => Ok(SchemaSource::Learned),
        other => Err(SqliteError::ConversionError(format!(
            "unknown schema source: {other}"
        ))),
    }
}

/// Inserts a [`CommandSchema`] into the commands table and returns the row ID.
pub fn insert_command(conn: &Connection, prefix: &str, schema: &CommandSchema) -> Result<i64> {
    conn.execute(
        &format!(
            "INSERT INTO {prefix}commands (name, description, version, source, confidence, schema_version) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
        ),
        params![
            schema.command,
            schema.description,
            schema.version,
            source_to_string(&schema.source),
            schema.confidence,
            schema.schema_version,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Inserts flags into the flags table with optional subcommand association.
///
/// Returns the insert counts and a scope-local map of canonical flag names to
/// row IDs. Relationships (conflicts/requires) are resolved using the local
/// map first, falling back to `global_flag_ids` for cross-scope references
/// (e.g. a subcommand flag that conflicts with a global flag).
pub fn insert_flags(
    conn: &Connection,
    prefix: &str,
    command_id: i64,
    subcommand_id: Option<i64>,
    flags: &[FlagSchema],
    global_flag_ids: &HashMap<String, i64>,
) -> Result<(InsertCounts, HashMap<String, i64>)> {
    let mut counts = InsertCounts::default();

    // First pass: insert all flags and build scope-local name-to-id mapping
    let mut local_flag_ids: HashMap<String, i64> = HashMap::new();

    for flag in flags {
        conn.execute(
            &format!(
                "INSERT INTO {prefix}flags (command_id, subcommand_id, short, long, value_type, takes_value, description, multiple) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
            ),
            params![
                command_id,
                subcommand_id,
                flag.short,
                flag.long,
                value_type_to_string(&flag.value_type),
                flag.takes_value as i32,
                flag.description,
                flag.multiple as i32,
            ],
        )?;
        let flag_id = conn.last_insert_rowid();
        counts.flags += 1;

        // Map canonical name to id for relationship resolution
        let canonical = flag.canonical_name().to_string();
        local_flag_ids.insert(canonical, flag_id);

        // Insert choices for Choice value types
        if let ValueType::Choice(ref choices) = flag.value_type {
            for choice in choices {
                conn.execute(
                    &format!(
                        "INSERT INTO {prefix}flag_choices (flag_id, choice) VALUES (?1, ?2)"
                    ),
                    params![flag_id, choice],
                )?;
                counts.choices += 1;
            }
        }
    }

    // Second pass: insert relationships using local map + global fallback
    for flag in flags {
        let canonical = flag.canonical_name().to_string();
        let flag_id = local_flag_ids[&canonical];

        for conflict in &flag.conflicts_with {
            if let Some(&related_id) = local_flag_ids
                .get(conflict)
                .or_else(|| global_flag_ids.get(conflict))
            {
                conn.execute(
                    &format!(
                        "INSERT INTO {prefix}flag_relationships (flag_id, related_flag_id, relationship_type) \
                         VALUES (?1, ?2, ?3)"
                    ),
                    params![flag_id, related_id, "conflicts"],
                )?;
                counts.relationships += 1;
            }
        }

        for required in &flag.requires {
            if let Some(&related_id) = local_flag_ids
                .get(required)
                .or_else(|| global_flag_ids.get(required))
            {
                conn.execute(
                    &format!(
                        "INSERT INTO {prefix}flag_relationships (flag_id, related_flag_id, relationship_type) \
                         VALUES (?1, ?2, ?3)"
                    ),
                    params![flag_id, related_id, "requires"],
                )?;
                counts.relationships += 1;
            }
        }
    }

    Ok((counts, local_flag_ids))
}

/// Recursively inserts subcommands with parent tracking.
///
/// For each subcommand, inserts aliases, flags, positional args, and
/// nested subcommands. The `global_flag_ids` map enables cross-scope
/// relationship resolution (e.g. a subcommand flag that conflicts with
/// a global flag).
pub fn insert_subcommands(
    conn: &Connection,
    prefix: &str,
    command_id: i64,
    parent_id: Option<i64>,
    subcommands: &[SubcommandSchema],
    global_flag_ids: &HashMap<String, i64>,
) -> Result<InsertCounts> {
    let mut counts = InsertCounts::default();

    for sub in subcommands {
        conn.execute(
            &format!(
                "INSERT INTO {prefix}subcommands (command_id, parent_id, name, description) \
                 VALUES (?1, ?2, ?3, ?4)"
            ),
            params![command_id, parent_id, sub.name, sub.description],
        )?;
        let sub_id = conn.last_insert_rowid();
        counts.subcommands += 1;

        // Insert aliases
        for alias in &sub.aliases {
            conn.execute(
                &format!(
                    "INSERT INTO {prefix}subcommand_aliases (subcommand_id, alias) VALUES (?1, ?2)"
                ),
                params![sub_id, alias],
            )?;
            counts.aliases += 1;
        }

        // Insert subcommand-scoped flags (with global fallback for relationships)
        let (flag_counts, _local_flag_ids) =
            insert_flags(conn, prefix, command_id, Some(sub_id), &sub.flags, global_flag_ids)?;
        counts.merge(&flag_counts);

        // Insert subcommand positional args
        let arg_counts =
            insert_positional_args(conn, prefix, None, Some(sub_id), &sub.positional)?;
        counts.merge(&arg_counts);

        // Recurse for nested subcommands
        let nested_counts =
            insert_subcommands(conn, prefix, command_id, Some(sub_id), &sub.subcommands, global_flag_ids)?;
        counts.merge(&nested_counts);
    }

    Ok(counts)
}

/// Inserts positional arguments into the positional_args table.
///
/// Each argument is stored with its position index (0-based). Choice values
/// are stored in the arg_choices table.
pub fn insert_positional_args(
    conn: &Connection,
    prefix: &str,
    command_id: Option<i64>,
    subcommand_id: Option<i64>,
    args: &[ArgSchema],
) -> Result<InsertCounts> {
    let mut counts = InsertCounts::default();

    for (position, arg) in args.iter().enumerate() {
        conn.execute(
            &format!(
                "INSERT INTO {prefix}positional_args (command_id, subcommand_id, position, name, value_type, required, multiple, description) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
            ),
            params![
                command_id,
                subcommand_id,
                position as i64,
                arg.name,
                value_type_to_string(&arg.value_type),
                arg.required as i32,
                arg.multiple as i32,
                arg.description,
            ],
        )?;
        let arg_id = conn.last_insert_rowid();
        counts.args += 1;

        // Insert choices for Choice value types
        if let ValueType::Choice(ref choices) = arg.value_type {
            for choice in choices {
                conn.execute(
                    &format!(
                        "INSERT INTO {prefix}arg_choices (arg_id, choice) VALUES (?1, ?2)"
                    ),
                    params![arg_id, choice],
                )?;
                counts.choices += 1;
            }
        }
    }

    Ok(counts)
}

/// Loads a complete [`CommandSchema`] from the database by command name.
///
/// Returns `None` if no command with the given name exists.
pub fn load_command(
    conn: &Connection,
    prefix: &str,
    command_name: &str,
) -> Result<Option<CommandSchema>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id, name, description, version, source, confidence, schema_version \
         FROM {prefix}commands WHERE name = ?1"
    ))?;

    let mut rows = stmt.query(params![command_name])?;
    let row = match rows.next()? {
        Some(row) => row,
        None => return Ok(None),
    };

    let command_id: i64 = row.get(0)?;
    let name: String = row.get(1)?;
    let description: Option<String> = row.get(2)?;
    let version: Option<String> = row.get(3)?;
    let source_str: String = row.get(4)?;
    let confidence: f64 = row.get(5)?;
    let schema_version: Option<String> = row.get(6)?;

    let source = string_to_source(&source_str)?;

    // Load global flags (where subcommand_id IS NULL)
    let global_flags = load_flags_for(conn, prefix, command_id, None)?;

    // Load top-level subcommands (where parent_id IS NULL)
    let subcommands = load_subcommands(conn, prefix, command_id, None)?;

    // Load global positional args (where subcommand_id IS NULL and command_id matches)
    let positional = load_positional_args(conn, prefix, Some(command_id), None)?;

    Ok(Some(CommandSchema {
        schema_version,
        command: name,
        description,
        global_flags,
        subcommands,
        positional,
        source,
        confidence,
        version,
    }))
}

/// Recursively loads subcommands for a given command and parent.
pub fn load_subcommands(
    conn: &Connection,
    prefix: &str,
    command_id: i64,
    parent_id: Option<i64>,
) -> Result<Vec<SubcommandSchema>> {
    // Collect raw row data first to avoid closure type mismatches
    let raw_rows: Vec<(i64, String, Option<String>)> = if let Some(pid) = parent_id {
        let mut stmt = conn.prepare(&format!(
            "SELECT id, name, description FROM {prefix}subcommands \
             WHERE command_id = ?1 AND parent_id = ?2 ORDER BY id"
        ))?;
        stmt.query_map(params![command_id, pid], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        let mut stmt = conn.prepare(&format!(
            "SELECT id, name, description FROM {prefix}subcommands \
             WHERE command_id = ?1 AND parent_id IS NULL ORDER BY id"
        ))?;
        stmt.query_map(params![command_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
    };

    let mut subcommands = Vec::new();
    for (sub_id, name, description) in raw_rows {
        // Load aliases
        let aliases = load_aliases(conn, prefix, sub_id)?;

        // Load subcommand-scoped flags
        let flags = load_flags_for(conn, prefix, command_id, Some(sub_id))?;

        // Load positional args
        let positional = load_positional_args(conn, prefix, None, Some(sub_id))?;

        // Recurse for nested subcommands
        let nested = load_subcommands(conn, prefix, command_id, Some(sub_id))?;

        subcommands.push(SubcommandSchema {
            name,
            description,
            flags,
            positional,
            subcommands: nested,
            aliases,
        });
    }

    Ok(subcommands)
}

/// Loads flags associated with a command/subcommand scope.
///
/// When `subcommand_id` is `None`, loads global flags (subcommand_id IS NULL).
/// When `subcommand_id` is `Some`, loads flags for that subcommand.
fn load_flags_for(
    conn: &Connection,
    prefix: &str,
    command_id: i64,
    subcommand_id: Option<i64>,
) -> Result<Vec<FlagSchema>> {
    type FlagRow = (i64, Option<String>, Option<String>, String, bool, Option<String>, bool);

    // Collect raw row data first to avoid closure type mismatches
    let raw_rows: Vec<FlagRow> = if let Some(sid) = subcommand_id {
        let mut stmt = conn.prepare(&format!(
            "SELECT id, short, long, value_type, takes_value, description, multiple \
             FROM {prefix}flags WHERE command_id = ?1 AND subcommand_id = ?2 ORDER BY id"
        ))?;
        stmt.query_map(params![command_id, sid], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, bool>(6)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        let mut stmt = conn.prepare(&format!(
            "SELECT id, short, long, value_type, takes_value, description, multiple \
             FROM {prefix}flags WHERE command_id = ?1 AND subcommand_id IS NULL ORDER BY id"
        ))?;
        stmt.query_map(params![command_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, bool>(6)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
    };

    let mut flags = Vec::new();
    for (flag_id, short, long, value_type_str, takes_value, description, multiple) in raw_rows {

        // Load choices for this flag
        let choices = load_flag_choices(conn, prefix, flag_id)?;

        let value_type = string_to_value_type(&value_type_str, choices)?;

        // Load relationships
        let (conflicts_with, requires) = load_flag_relationships(conn, prefix, flag_id)?;

        flags.push(FlagSchema {
            short,
            long,
            value_type,
            takes_value,
            description,
            multiple,
            conflicts_with,
            requires,
        });
    }

    Ok(flags)
}

/// Loads choices for a specific flag from the flag_choices table.
fn load_flag_choices(conn: &Connection, prefix: &str, flag_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT choice FROM {prefix}flag_choices WHERE flag_id = ?1 ORDER BY id"
    ))?;
    let rows = stmt.query_map(params![flag_id], |row| row.get::<_, String>(0))?;
    let mut choices = Vec::new();
    for row in rows {
        choices.push(row?);
    }
    Ok(choices)
}

/// Loads choices for a specific arg from the arg_choices table.
fn load_arg_choices(conn: &Connection, prefix: &str, arg_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT choice FROM {prefix}arg_choices WHERE arg_id = ?1 ORDER BY id"
    ))?;
    let rows = stmt.query_map(params![arg_id], |row| row.get::<_, String>(0))?;
    let mut choices = Vec::new();
    for row in rows {
        choices.push(row?);
    }
    Ok(choices)
}

/// Loads flag relationships (conflicts_with and requires) for a flag.
///
/// Returns a tuple of (conflicts_with, requires) where each is a vector
/// of canonical flag names.
fn load_flag_relationships(
    conn: &Connection,
    prefix: &str,
    flag_id: i64,
) -> Result<(Vec<String>, Vec<String>)> {
    let mut stmt = conn.prepare(&format!(
        "SELECT r.related_flag_id, r.relationship_type, f.short, f.long \
         FROM {prefix}flag_relationships r \
         JOIN {prefix}flags f ON f.id = r.related_flag_id \
         WHERE r.flag_id = ?1 ORDER BY r.id"
    ))?;

    let rows = stmt.query_map(params![flag_id], |row| {
        Ok((
            row.get::<_, String>(1)?, // relationship_type
            row.get::<_, Option<String>>(2)?, // short
            row.get::<_, Option<String>>(3)?, // long
        ))
    })?;

    let mut conflicts_with = Vec::new();
    let mut requires = Vec::new();

    for row in rows {
        let (rel_type, short, long) = row?;
        // Use long form preferred, fallback to short
        let name = long.or(short).unwrap_or_else(|| "unknown".to_string());
        match rel_type.as_str() {
            "conflicts" => conflicts_with.push(name),
            "requires" => requires.push(name),
            _ => {}
        }
    }

    Ok((conflicts_with, requires))
}

/// Loads aliases for a subcommand.
fn load_aliases(conn: &Connection, prefix: &str, subcommand_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT alias FROM {prefix}subcommand_aliases WHERE subcommand_id = ?1 ORDER BY id"
    ))?;
    let rows = stmt.query_map(params![subcommand_id], |row| row.get::<_, String>(0))?;
    let mut aliases = Vec::new();
    for row in rows {
        aliases.push(row?);
    }
    Ok(aliases)
}

/// Loads positional arguments for a command or subcommand.
fn load_positional_args(
    conn: &Connection,
    prefix: &str,
    command_id: Option<i64>,
    subcommand_id: Option<i64>,
) -> Result<Vec<ArgSchema>> {
    let mut stmt = match (command_id, subcommand_id) {
        (Some(_), None) => conn.prepare(&format!(
            "SELECT id, name, value_type, required, multiple, description \
             FROM {prefix}positional_args \
             WHERE command_id = ?1 AND subcommand_id IS NULL \
             ORDER BY position"
        ))?,
        (None, Some(_)) => conn.prepare(&format!(
            "SELECT id, name, value_type, required, multiple, description \
             FROM {prefix}positional_args \
             WHERE subcommand_id = ?1 AND command_id IS NULL \
             ORDER BY position"
        ))?,
        _ => return Ok(Vec::new()),
    };

    let id_param = command_id.or(subcommand_id).unwrap();
    let rows = stmt.query_map(params![id_param], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, bool>(3)?,
            row.get::<_, bool>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })?;

    let mut args = Vec::new();
    for row in rows {
        let (arg_id, name, value_type_str, required, multiple, description) = row?;

        let choices = load_arg_choices(conn, prefix, arg_id)?;
        let value_type = string_to_value_type(&value_type_str, choices)?;

        args.push(ArgSchema {
            name,
            value_type,
            required,
            multiple,
            description,
        });
    }

    Ok(args)
}

/// Counts of items inserted during a conversion operation.
#[derive(Debug, Default)]
pub struct InsertCounts {
    pub flags: usize,
    pub subcommands: usize,
    pub args: usize,
    pub choices: usize,
    pub aliases: usize,
    pub relationships: usize,
}

impl InsertCounts {
    /// Merges another set of counts into this one.
    pub fn merge(&mut self, other: &InsertCounts) {
        self.flags += other.flags;
        self.subcommands += other.subcommands;
        self.args += other.args;
        self.choices += other.choices;
        self.aliases += other.aliases;
        self.relationships += other.relationships;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_type_round_trip() {
        let types = vec![
            ValueType::Bool,
            ValueType::String,
            ValueType::Number,
            ValueType::File,
            ValueType::Directory,
            ValueType::Url,
            ValueType::Branch,
            ValueType::Remote,
            ValueType::Any,
        ];

        for vt in types {
            let s = value_type_to_string(&vt);
            let restored = string_to_value_type(&s, vec![]).unwrap();
            assert_eq!(vt, restored);
        }
    }

    #[test]
    fn test_choice_value_type_round_trip() {
        let choices = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let vt = ValueType::Choice(choices.clone());
        let s = value_type_to_string(&vt);
        assert_eq!(s, "Choice");
        let restored = string_to_value_type(&s, choices.clone()).unwrap();
        assert_eq!(restored, ValueType::Choice(choices));
    }

    #[test]
    fn test_source_round_trip() {
        let sources = vec![
            SchemaSource::HelpCommand,
            SchemaSource::ManPage,
            SchemaSource::Bootstrap,
            SchemaSource::Learned,
        ];

        for source in sources {
            let s = source_to_string(&source);
            let restored = string_to_source(s).unwrap();
            assert_eq!(source, restored);
        }
    }

    #[test]
    fn test_unknown_value_type() {
        assert!(string_to_value_type("UnknownType", vec![]).is_err());
    }

    #[test]
    fn test_unknown_source() {
        assert!(string_to_source("UnknownSource").is_err());
    }
}
