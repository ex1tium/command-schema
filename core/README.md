# command-schema-core

Core types and validation for CLI command schemas.

This crate provides the foundational type system for representing CLI command structures. It has no I/O dependencies and is designed as a pure data layer.

## Key Types

- `CommandSchema` - Main schema structure describing a CLI command
- `FlagSchema` - Flag and option definitions (short/long forms, value types)
- `ArgSchema` - Positional argument definitions
- `SubcommandSchema` - Subcommand definitions with nested flags and args
- `SchemaPackage` - Bundle multiple schemas together

## Features

- Schema validation via `validate_schema()` and `validate_package()`
- Schema merging with `merge_schemas()` and `MergeStrategy`
- Serde serialization/deserialization
- No I/O dependencies (pure types)

## Quick Example

```rust
use command_schema_core::{CommandSchema, FlagSchema, SchemaSource};

let mut schema = CommandSchema::new("mycli", SchemaSource::Bootstrap);
schema.description = Some("My CLI tool".to_string());
schema.global_flags.push(
    FlagSchema::boolean(Some("-v"), Some("--verbose"))
        .with_description("Enable verbose output")
);
```

## Documentation

See the [repository root](https://github.com/ex1tium/command-schema) for comprehensive documentation and usage examples.

## License

MIT
