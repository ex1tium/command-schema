# command-schema-discovery

Discover and extract CLI command schemas from help output.

This crate provides a library for parsing help text from CLI commands and extracting structured schemas. It supports multiple help format conventions and includes confidence scoring and quality metrics.

## Key Components

- `HelpParser` - Multi-strategy help text parser
- `CommandExtractor` - Extract schemas by executing commands
- `SchemaCache` - Fingerprint-based caching
- `parse_help_text()` - Convenience function for parsing

## Supported Formats

- GNU style (`--long`, `-s`)
- Clap (Rust) / Cobra (Go) / Argparse (Python) style
- NPM-style subcommand listings
- BSD-style flags
- Generic section-based help

## Quick Example

```rust
use command_schema_discovery::parse_help_text;

let result = parse_help_text("git", include_str!("git-help.txt"));
if let Some(schema) = result.schema {
    println!("Found {} subcommands", schema.subcommands.len());
}
```

## Features

- Multiple parsing strategies with automatic detection
- Confidence scoring and quality metrics
- Extraction reports with diagnostics
- Optional `clap` feature for CLI integration

## Documentation

See the [repository root](https://github.com/ex1tium/command-schema) for CLI usage and integration examples.

## License

MIT
