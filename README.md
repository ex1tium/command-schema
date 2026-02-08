# command-schema

Extract structured schemas from CLI `--help` output.

Given a command's help text, `command-schema` parses it into a typed schema describing flags, positional arguments, subcommands, and their relationships.

## Crates

| Crate | Description |
|-------|-------------|
| [`command-schema-core`](core/) | Core types: `CommandSchema`, `FlagSchema`, `ArgSchema`, `SubcommandSchema`, validation, merging |
| [`command-schema-discovery`](discovery/) | Help text parser, command probing, extraction CLI (`schema-discover`), caching, reporting |

## Quick start

### As a library

```rust
use command_schema_discovery::parse_help_text;

let result = parse_help_text("ls", include_str!("help-output.txt"));
if let Some(schema) = result.schema {
    for flag in &schema.global_flags {
        println!("{:?} — {}", flag.long, flag.description);
    }
}
```

### As a CLI

```bash
# Parse help from a file
schema-discover parse-file --command ls --path ls-help.txt --format json

# Parse help from stdin
curl --help | schema-discover parse-stdin --command curl

# Extract schemas from installed commands
schema-discover extract --installed-only --jobs 4
```

## Supported formats

The parser detects and handles multiple help output conventions:

- GNU (`--long-flag`, `-s` short flags)
- Clap / Cobra / Argparse style
- NPM-style subcommand listings
- BSD-style flags
- Generic section-based help

## License

MIT
