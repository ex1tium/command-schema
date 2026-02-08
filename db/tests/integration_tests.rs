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
