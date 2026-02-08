# command-schema-sqlite

SQLite storage backend for command schemas with normalized tables and migration support.

This crate provides relational storage for command schemas with a full migration lifecycle, CRUD operations, and customizable table prefixes.

## Key Components

- `Migration` - Database lifecycle (up, down, seed, refresh, status)
- `SchemaQuery` - CRUD operations for schemas
- Normalized schema with 8 tables
- Customizable table prefixes

## Database Schema

- `{prefix}commands` - Command metadata
- `{prefix}flags` - Global and subcommand-scoped flags
- `{prefix}subcommands` - Subcommand hierarchy
- `{prefix}positional_args` - Positional arguments
- `{prefix}flag_choices` - Enum values for flags
- `{prefix}arg_choices` - Enum values for arguments
- `{prefix}subcommand_aliases` - Subcommand aliases
- `{prefix}flag_relationships` - Conflicts and requirements

## Quick Example

```rust,no_run
use command_schema_sqlite::{Migration, SchemaQuery};
use rusqlite::Connection;

// Set up database
let conn = Connection::open("schemas.db")?;
let mut migration = Migration::new(conn, "cs_")?;
migration.up()?;
migration.seed("schemas/database/")?;

// Query schemas
let conn = migration.into_connection();
let query = SchemaQuery::new(conn, "cs_")?;
if let Some(schema) = query.get_schema("git")? {
    println!("Found git schema");
}
```

## Features

- Full migration lifecycle with up/down/seed/refresh
- Normalized storage with foreign key constraints
- Customizable table prefixes for multi-tenant use
- Transaction-safe operations
- Cascading deletes

## Documentation

See the [repository root](https://github.com/ex1tium/command-schema) for integration examples.

## License

MIT
