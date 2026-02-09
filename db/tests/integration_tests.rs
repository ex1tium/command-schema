use std::io::Write;
use std::path::Path;

use command_schema_core::{CommandSchema, SchemaPackage, SchemaSource};
use command_schema_db::{
    CIConfig, CommandMetadata, Manifest, QualityPolicyFingerprint, SchemaDatabase,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_schema(name: &str) -> CommandSchema {
    CommandSchema::new(name, SchemaSource::Bootstrap)
}

fn write_schema(dir: &Path, schema: &CommandSchema) {
    let path = dir.join(format!("{}.json", schema.command));
    let mut f = std::fs::File::create(path).unwrap();
    serde_json::to_writer_pretty(&mut f, schema).unwrap();
    f.flush().unwrap();
}

fn sample_metadata(version: Option<&str>, checksum: &str) -> CommandMetadata {
    CommandMetadata {
        version: version.map(String::from),
        executable_path: Some("/usr/bin/test".into()),
        mtime_secs: Some(1_700_000_000),
        size_bytes: Some(100_000),
        extracted_at: "2024-01-15T10:30:00Z".into(),
        quality_tier: "high".into(),
        checksum: checksum.into(),
        implementation: Some("test".into()),
        schema_file: Some("test.json".into()),
    }
}

// ---------------------------------------------------------------------------
// Directory loading
// ---------------------------------------------------------------------------

#[test]
fn test_directory_loading() {
    let dir = std::env::temp_dir().join("cs_db_integ_dir");
    std::fs::create_dir_all(&dir).unwrap();

    write_schema(&dir, &test_schema("git"));
    write_schema(&dir, &test_schema("docker"));
    write_schema(&dir, &test_schema("cargo"));
    write_schema(&dir, &test_schema("npm"));
    write_schema(&dir, &test_schema("rustup"));

    let db = SchemaDatabase::from_dir(&dir).unwrap();
    assert_eq!(db.len(), 5);
    assert!(db.contains("git"));
    assert!(db.contains("docker"));
    assert!(db.contains("cargo"));
    assert!(db.contains("npm"));
    assert!(db.contains("rustup"));

    let git = db.get("git").unwrap();
    assert_eq!(git.command, "git");

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Bundle loading
// ---------------------------------------------------------------------------

#[test]
fn test_bundle_loading() {
    let dir = std::env::temp_dir().join("cs_db_integ_bundle");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("schemas.json");

    let mut package = SchemaPackage::new("1.0.0", "2024-01-01T00:00:00Z");
    package.schemas.push(test_schema("git"));
    package.schemas.push(test_schema("docker"));
    package.schemas.push(test_schema("cargo"));

    let mut f = std::fs::File::create(&path).unwrap();
    serde_json::to_writer_pretty(&mut f, &package).unwrap();
    f.flush().unwrap();

    let db = SchemaDatabase::from_bundle(&path).unwrap();
    assert_eq!(db.len(), 3);
    assert!(db.contains("git"));
    assert!(db.contains("docker"));
    assert!(db.contains("cargo"));

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Builder fallback chain
// ---------------------------------------------------------------------------

#[test]
fn test_builder_fallback_to_directory() {
    let dir = std::env::temp_dir().join("cs_db_integ_builder_dir");
    std::fs::create_dir_all(&dir).unwrap();
    write_schema(&dir, &test_schema("git"));

    // Bundled not available (feature not enabled in default test), directory should win.
    let db = SchemaDatabase::builder()
        .with_bundled()
        .from_dir(&dir)
        .build()
        .unwrap();

    assert!(db.contains("git"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_builder_fallback_to_bundle() {
    let dir = std::env::temp_dir().join("cs_db_integ_builder_bundle");
    std::fs::create_dir_all(&dir).unwrap();
    let bundle_path = dir.join("schemas.json");

    let mut package = SchemaPackage::new("1.0.0", "2024-01-01T00:00:00Z");
    package.schemas.push(test_schema("docker"));

    let mut f = std::fs::File::create(&bundle_path).unwrap();
    serde_json::to_writer_pretty(&mut f, &package).unwrap();
    f.flush().unwrap();

    let db = SchemaDatabase::builder()
        .from_dir("/nonexistent/integ_test_dir/")
        .from_bundle(&bundle_path)
        .build()
        .unwrap();

    assert!(db.contains("docker"));

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Manifest workflow
// ---------------------------------------------------------------------------

#[test]
fn test_manifest_workflow() {
    let dir = std::env::temp_dir().join("cs_db_integ_manifest");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("manifest.json");

    // Create with 3 commands.
    let mut manifest = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
    manifest.update_entry("git".into(), sample_metadata(Some("2.43.0"), "aaa"));
    manifest.update_entry("docker".into(), sample_metadata(Some("24.0.7"), "bbb"));
    manifest.update_entry("cargo".into(), sample_metadata(Some("1.75.0"), "ccc"));

    // Save and reload.
    manifest.save(&path).unwrap();
    let loaded = Manifest::load(&path).unwrap();
    assert_eq!(loaded.commands.len(), 3);
    assert!(loaded.contains("git"));
    assert!(loaded.contains("docker"));
    assert!(loaded.contains("cargo"));
    assert_eq!(
        loaded.get("git").unwrap().version.as_deref(),
        Some("2.43.0")
    );

    // Diff: update one entry.
    let mut updated = loaded.clone();
    updated.update_entry("git".into(), sample_metadata(Some("2.44.0"), "ddd"));

    let diff = loaded.diff(&updated);
    assert!(diff.contains(&"git".to_string()));
    // docker and cargo unchanged.
    assert!(!diff.contains(&"docker".to_string()));
    assert!(!diff.contains(&"cargo".to_string()));

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Bundled schemas (feature-gated tests)
// ---------------------------------------------------------------------------

#[cfg(feature = "bundled-schemas")]
mod bundled_schemas {
    use command_schema_core::CommandSchema;
    use command_schema_db::{SchemaDatabase, load_bundled_schemas};
    use std::io::Write;
    use std::path::Path;

    fn write_schema(dir: &Path, schema: &CommandSchema) {
        let path = dir.join(format!("{}.json", schema.command));
        let mut f = std::fs::File::create(path).unwrap();
        serde_json::to_writer_pretty(&mut f, schema).unwrap();
        f.flush().unwrap();
    }

    #[test]
    fn test_bundled_schemas_roundtrip() {
        let schema_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("schemas")
            .join("database");

        let bundled_schemas = load_bundled_schemas().unwrap();
        if bundled_schemas.is_empty() && !schema_dir.is_dir() {
            // No schemas bundled and no source directory; nothing to test.
            return;
        }

        assert!(
            !bundled_schemas.is_empty(),
            "schemas/database/ exists but no schemas were bundled"
        );

        for bundled in &bundled_schemas {
            let source_path = schema_dir.join(format!("{}.json", bundled.command));
            assert!(
                source_path.exists(),
                "Source JSON file missing for bundled schema '{}'",
                bundled.command
            );

            let file = std::fs::File::open(&source_path).unwrap();
            let reader = std::io::BufReader::new(file);
            let source: CommandSchema = serde_json::from_reader(reader).unwrap();

            // Serialize both to canonical (sorted-key, deterministic) JSON and
            // compare strings so every field — positional args, descriptions,
            // nested subcommands, etc. — is covered.
            let bundled_json = serde_json::to_string(&bundled).unwrap();
            let source_json = serde_json::to_string(&source).unwrap();

            assert_eq!(
                bundled_json, source_json,
                "Full schema mismatch for '{}': bundled schema does not equal source JSON",
                bundled.command
            );
        }
    }

    #[test]
    fn test_bundled_schemas_count() {
        let bundled_schemas = load_bundled_schemas().unwrap();

        let schema_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("schemas")
            .join("database");

        if !schema_dir.is_dir() {
            // No schema directory; bundled should be empty
            assert!(bundled_schemas.is_empty());
            return;
        }

        let json_count = std::fs::read_dir(&schema_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let path = e.path();
                path.extension().and_then(|ext| ext.to_str()) == Some("json")
                    && path.file_stem().and_then(|s| s.to_str()) != Some("manifest")
            })
            .count();

        assert_eq!(
            bundled_schemas.len(),
            json_count,
            "Bundled schema count ({}) doesn't match JSON file count ({})",
            bundled_schemas.len(),
            json_count
        );
    }

    #[test]
    fn test_builder_with_bundled_priority() {
        let bundled_db = SchemaDatabase::bundled().unwrap();
        if bundled_db.is_empty() {
            // No bundled schemas available; skip test
            return;
        }

        // Pick a command name from the bundled set
        let bundled_command = bundled_db.commands().next().unwrap().to_string();

        // Create a temp directory with a modified schema for the same command
        let dir = std::env::temp_dir().join("cs_db_integ_bundled_priority");
        std::fs::create_dir_all(&dir).unwrap();

        let mut modified = CommandSchema::new(
            &bundled_command,
            command_schema_core::SchemaSource::Bootstrap,
        );
        modified.description = Some("MODIFIED_FOR_TEST".into());
        write_schema(&dir, &modified);

        // Builder: bundled first, then directory
        let db = SchemaDatabase::builder()
            .with_bundled()
            .from_dir(&dir)
            .build()
            .unwrap();

        // Bundled should win — description should NOT be our modified marker
        let schema = db.get(&bundled_command).unwrap();
        assert_ne!(
            schema.description.as_deref(),
            Some("MODIFIED_FOR_TEST"),
            "Directory schema should NOT override bundled schema"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}

// ---------------------------------------------------------------------------
// Performance validation (run with --ignored --nocapture)
// ---------------------------------------------------------------------------

mod performance_validation {
    use command_schema_db::SchemaDatabase;
    use std::path::Path;

    #[test]
    #[ignore]
    fn test_startup_time_constraint() {
        let schema_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("schemas")
            .join("database");

        if !schema_dir.is_dir() {
            println!("SKIP: schemas/database/ directory not found");
            return;
        }

        let start = std::time::Instant::now();
        let db = SchemaDatabase::from_dir(&schema_dir).unwrap();
        let elapsed = start.elapsed();

        let elapsed_ms = elapsed.as_millis();
        println!(
            "Directory loading: {} schemas in {}ms",
            db.len(),
            elapsed_ms
        );

        assert!(
            elapsed_ms < 100,
            "Startup time {}ms exceeds 100ms constraint for {} schemas",
            elapsed_ms,
            db.len()
        );
    }

    #[test]
    #[ignore]
    fn test_memory_usage_constraint() {
        let schema_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("schemas")
            .join("database");

        if !schema_dir.is_dir() {
            println!("SKIP: schemas/database/ directory not found");
            return;
        }

        let db = SchemaDatabase::from_dir(&schema_dir).unwrap();

        // Estimate memory by serializing to JSON
        let mut total_bytes = 0usize;
        for name in db.commands() {
            if let Some(schema) = db.get(name) {
                if let Ok(json) = serde_json::to_string(schema) {
                    total_bytes += json.len();
                }
            }
        }
        let memory_mb = total_bytes as f64 / (1024.0 * 1024.0);

        println!(
            "Memory usage: {:.3} MB for {} schemas ({} bytes)",
            memory_mb,
            db.len(),
            total_bytes,
        );

        assert!(
            memory_mb < 10.0,
            "Memory usage {:.3} MB exceeds 10 MB constraint for {} schemas",
            memory_mb,
            db.len()
        );
    }

    #[cfg(feature = "bundled-schemas")]
    #[test]
    #[ignore]
    fn test_bundled_startup_time() {
        let schema_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("schemas")
            .join("database");

        // Measure directory loading time
        let dir_start = std::time::Instant::now();
        let dir_db = SchemaDatabase::from_dir(&schema_dir).unwrap();
        let dir_elapsed = dir_start.elapsed();

        // Measure bundled loading time
        let bundled_start = std::time::Instant::now();
        let bundled_db = SchemaDatabase::bundled().unwrap();
        let bundled_elapsed = bundled_start.elapsed();

        println!(
            "Directory loading: {} schemas in {:.2?}",
            dir_db.len(),
            dir_elapsed
        );
        println!(
            "Bundled loading:   {} schemas in {:.2?}",
            bundled_db.len(),
            bundled_elapsed
        );

        if !bundled_db.is_empty() {
            println!(
                "Speedup: {:.1}x",
                dir_elapsed.as_secs_f64() / bundled_elapsed.as_secs_f64()
            );

            assert!(
                bundled_elapsed.as_millis() < 100,
                "Bundled startup time {}ms exceeds 100ms constraint",
                bundled_elapsed.as_millis()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// CI config workflow
// ---------------------------------------------------------------------------

#[test]
fn test_ci_config_workflow() {
    let dir = std::env::temp_dir().join("cs_db_integ_ci_config");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yml");

    let yaml = r#"
version: "1.0"
allowlist:
  - git
  - docker
  - cargo
exclude:
  - dangerous-tool
  - broken-cmd
quality:
  min_confidence: 0.7
  min_coverage: 0.3
  allow_low_quality: false
extraction:
  jobs: 8
  installed_only: true
  scan_path: false
"#;
    std::fs::write(&path, yaml).unwrap();

    let config = CIConfig::load(&path).unwrap();
    assert_eq!(config.version, "1.0");
    assert_eq!(config.allowlist.len(), 3);
    assert!(config.is_allowed("git"));
    assert!(config.is_allowed("docker"));
    assert!(!config.is_allowed("unknown"));
    assert!(config.is_excluded("dangerous-tool"));
    assert!(config.is_excluded("broken-cmd"));
    assert!(!config.is_excluded("git"));

    // Save and reload.
    let path2 = dir.join("config2.yml");
    config.save(&path2).unwrap();
    let reloaded = CIConfig::load(&path2).unwrap();
    assert_eq!(reloaded.allowlist, config.allowlist);
    assert_eq!(reloaded.exclude, config.exclude);

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Edge case tests
// ---------------------------------------------------------------------------

#[test]
fn test_empty_directory_loading() {
    let dir = std::env::temp_dir().join("cs_db_integ_empty_dir");
    std::fs::create_dir_all(&dir).unwrap();

    // Remove any leftover files from previous runs
    for entry in std::fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        std::fs::remove_file(entry.path()).ok();
    }

    let db = SchemaDatabase::from_dir(&dir).unwrap();
    assert!(db.is_empty());
    assert_eq!(db.len(), 0);
    assert!(db.get("nonexistent").is_none());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_corrupt_json_file_handling() {
    let dir = std::env::temp_dir().join("cs_db_integ_corrupt_json");
    std::fs::create_dir_all(&dir).unwrap();

    // Write a valid schema
    write_schema(&dir, &test_schema("git"));

    // Write a corrupt JSON file
    let corrupt_path = dir.join("broken.json");
    std::fs::write(&corrupt_path, "{ this is not valid json }}}").unwrap();

    // Loading should fail due to the corrupt file
    let result = SchemaDatabase::from_dir(&dir);
    assert!(
        result.is_err(),
        "Loading directory with corrupt JSON should fail"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_nonexistent_directory_error() {
    let result = SchemaDatabase::from_dir("/nonexistent/path/that/does/not/exist");
    assert!(
        result.is_err(),
        "Loading from nonexistent directory should fail"
    );
}

#[test]
fn test_manifest_diff_with_empty_manifests() {
    let m1 = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
    let m2 = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());

    let diff = m1.diff(&m2);
    assert!(
        diff.is_empty(),
        "Diff of two empty manifests should be empty"
    );
}

#[test]
fn test_manifest_diff_detects_new_commands() {
    let m1 = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
    let mut m2 = Manifest::new("0.1.0".into(), QualityPolicyFingerprint::default());
    m2.update_entry("git".into(), sample_metadata(Some("2.43.0"), "aaa"));

    let diff = m1.diff(&m2);
    assert!(
        diff.contains(&"git".to_string()),
        "Diff should detect newly added commands"
    );
}

#[test]
fn test_builder_all_sources_fail() {
    let result = SchemaDatabase::builder()
        .from_dir("/nonexistent/a/")
        .from_dir("/nonexistent/b/")
        .build();

    assert!(
        result.is_err(),
        "Builder with all failing sources should return error"
    );
}
