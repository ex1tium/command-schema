# command-schema

Parse CLI help text into structured schemas, then consume those schemas from JSON files, bundles, embedded data, or SQLite.

## Workspace Crates

| Crate | Purpose | README |
| --- | --- | --- |
| `command-schema-core` | Core types, validation, merge utilities | [core/README.md](core/README.md) |
| `command-schema-discovery` | Help-text parser and extraction logic | [discovery/README.md](discovery/README.md) |
| `command-schema-db` | In-memory schema DB + manifests + bundled schemas | [db/README.md](db/README.md) |
| `command-schema-sqlite` | SQLite migrations and CRUD query layer | [sqlite/README.md](sqlite/README.md) |
| `command-schema-cli` (`schema-discover`) | CLI for extraction, validation, bundling, migrations | [cli/README.md](cli/README.md) |

## Quick Start

### Parse help text (library)

```rust
use command_schema_discovery::parse_help_text;

let result = parse_help_text("git", include_str!("git-help.txt"));
if let Some(schema) = result.schema {
    println!("subcommands: {}", schema.subcommands.len());
}
```

### Use the CLI

```bash
cargo run -p command-schema-cli -- parse-file --command git --input git-help.txt
cargo run -p command-schema-cli -- extract --installed-only --output ./schemas
```

### Load pre-extracted schemas

```rust,no_run
use command_schema_db::SchemaDatabase;

let db = SchemaDatabase::from_dir("schemas/database/")?;
if let Some(schema) = db.get("git") {
    println!("flags: {}", schema.global_flags.len());
}
```

## Docs

- Integration patterns: [docs/integration-guide.md](docs/integration-guide.md)
- Schema contract: [docs/schema-contract.md](docs/schema-contract.md)
- Discovery production notes: [docs/command-schema-discovery-production-plan.md](docs/command-schema-discovery-production-plan.md)

## Repo Resources

- Pre-extracted schemas: [schemas/database/](schemas/database/)
- Schema JSON Schemas: [schemas/json-schema/](schemas/json-schema/)
- Command source lists: [schemas/command-lists/](schemas/command-lists/)
- CI config example: [ci-config.yaml](ci-config.yaml)
- Runnable examples: [examples/](examples/)
- Change history: [CHANGELOG.md](CHANGELOG.md)

## Development

```bash
cargo test
cargo test --lib
```

License: [MIT](LICENSE)
