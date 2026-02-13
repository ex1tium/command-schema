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
- Man pages:
  - Raw roff source (`mdoc` and legacy `man` macros)
  - Rendered manual output (e.g. `GIT-REBASE(1)` with `NAME`/`SYNOPSIS`/`OPTIONS` sections)
- Generic section-based help

When man-page structure is detected, man parsing is used as the primary extraction source.
If man extraction yields insufficient entities, parser fallbacks continue with other strategies.
Raw roff extraction is treated as high-confidence due to explicit macro structure.

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
