# command-schema-db

Static database loading and manifest management for command schemas.

This crate provides in-memory schema storage with multiple loading strategies, manifest-based version tracking, and CI extraction configuration.

## Key Components

- `SchemaDatabase` - In-memory HashMap with O(1) lookups
- `DatabaseBuilder` - Fluent API with fallback chain
- `Manifest` - Version tracking and checksum management
- `CIConfig` - CI extraction configuration

## Loading Patterns

- Directory loading: `SchemaDatabase::from_dir()`
- Bundle loading: `SchemaDatabase::from_bundle()`
- Bundled schemas: `SchemaDatabase::with_bundled()` (feature-gated)
- Builder pattern: `SchemaDatabase::builder().from_dir().with_bundled().build()`

## Quick Example

```rust,no_run
use command_schema_db::SchemaDatabase;

// Simple directory loading
let db = SchemaDatabase::from_dir("schemas/database/")?;
if let Some(schema) = db.get("git") {
    println!("git has {} flags", schema.global_flags.len());
}

// Builder with fallback chain
let db = SchemaDatabase::builder()
    .with_bundled()                // Try bundled first
    .from_dir("schemas/database/") // Fallback to directory
    .build()?;
```

## Features

- Optional `bundled-schemas` feature for zero-I/O access
- Manifest-based version tracking for CI efficiency
- CI configuration via YAML
- Checksum validation

## Performance Characteristics

The `bundled-schemas` feature embeds gzip-compressed schemas at build time for zero-I/O startup.

### Startup Time

| Source | Schemas | Time |
|--------|---------|------|
| Directory loading | ~107 | ~20-50ms |
| Bundled loading | ~107 | ~5-15ms |
| **Target** | **200** | **<100ms** |

### Memory Usage

| Metric | Value |
|--------|-------|
| In-memory HashMap | ~2-5 MB for 107 schemas |
| **Target** | **<10 MB for 200 schemas** |

### Binary Size Impact

| Build | Size |
|-------|------|
| Without `bundled-schemas` | baseline |
| With `bundled-schemas` | baseline + ~1-3 MB |
| **Target increase** | **<5 MB for 200 schemas** |

### Compression Ratio

Schemas are compressed with gzip at default compression level, typically achieving 70-85% size reduction from raw JSON.

### Measuring Performance

```bash
# Measure binary size impact
cargo build --release -p command-schema-examples --example wrashpty_integration
ls -lh target/release/examples/wrashpty_integration

cargo build --release -p command-schema-examples --example wrashpty_integration --features bundled-schemas
ls -lh target/release/examples/wrashpty_integration

# Run performance validation tests
cargo test -p command-schema-db --release --features bundled-schemas -- --ignored --nocapture performance_validation

# Run wrashpty integration example with metrics
cargo run -p command-schema-examples --example wrashpty_integration --features bundled-schemas
```

## Documentation

See the [repository root](https://github.com/ex1tium/command-schema) for CI automation details and integration examples.

## License

MIT
