use std::fs;
use std::path::PathBuf;

/// Helper to create a temp directory that is cleaned up on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("schema_cli_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("failed to create temp dir");
        Self { path }
    }

    fn path(&self) -> &PathBuf {
        &self.path
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Minimal CI config YAML for testing.
fn write_ci_config(dir: &TempDir, commands: &[&str]) -> PathBuf {
    let allowlist: Vec<String> = commands.iter().map(|c| format!("  - {c}")).collect();
    let yaml = format!(
        r#"version: "1.0"
allowlist:
{allowlist}
exclude: []
quality:
  min_confidence: 0.1
  min_coverage: 0.1
  allow_low_quality: true
extraction:
  jobs: 2
  installed_only: true
  scan_path: false
"#,
        allowlist = allowlist.join("\n")
    );
    let path = dir.join("ci-config.yaml");
    fs::write(&path, yaml).expect("failed to write ci config");
    path
}

/// Minimal CommandSchema JSON for seeding tests.
fn write_test_schema(dir: &TempDir, command: &str) -> PathBuf {
    let json = serde_json::json!({
        "schema_version": "1.0",
        "command": command,
        "source": "help",
        "description": format!("Test schema for {command}"),
        "global_flags": [],
        "subcommands": [],
        "positional": []
    });
    let path = dir.join(format!("{command}.json"));
    fs::write(&path, serde_json::to_string_pretty(&json).unwrap())
        .expect("failed to write schema");
    path
}

// ---------------------------------------------------------------------------
// CI Extract tests
// ---------------------------------------------------------------------------

#[test]
fn ci_extract_creates_manifest_on_first_run() {
    let dir = TempDir::new("ci_manifest_create");
    let output = TempDir::new("ci_manifest_create_out");
    // Use a known-installed command (echo is almost always available)
    let config_path = write_ci_config(&dir, &["echo"]);
    let manifest_path = dir.join("manifest.json");

    assert!(!manifest_path.exists());

    let status = std::process::Command::new(env!("CARGO_BIN_EXE_schema-discover"))
        .args([
            "ci-extract",
            "--config",
            config_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--output",
            output.path().to_str().unwrap(),
        ])
        .status()
        .expect("failed to run schema-discover");

    // The command should succeed (exit 0) regardless of whether echo schema is extractable
    assert!(status.success(), "ci-extract should succeed");
    // Manifest should now exist
    assert!(manifest_path.exists(), "manifest.json should be created");
}

#[test]
fn ci_extract_skips_unchanged_commands() {
    let dir = TempDir::new("ci_skip_unchanged");
    let output = TempDir::new("ci_skip_unchanged_out");
    let config_path = write_ci_config(&dir, &["echo"]);
    let manifest_path = dir.join("manifest.json");
    let bin = env!("CARGO_BIN_EXE_schema-discover");

    // First run — extracts
    let status = std::process::Command::new(bin)
        .args([
            "ci-extract",
            "--config",
            config_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--output",
            output.path().to_str().unwrap(),
        ])
        .output()
        .expect("failed to run schema-discover");

    assert!(status.status.success());
    let first_manifest = fs::read_to_string(&manifest_path).unwrap();

    // Second run — should skip since nothing changed
    let output2 = std::process::Command::new(bin)
        .args([
            "ci-extract",
            "--config",
            config_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--output",
            output.path().to_str().unwrap(),
        ])
        .output()
        .expect("failed to run schema-discover");

    assert!(output2.status.success());
    let stdout = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout.contains("Skipped: ") || stdout.contains("Extracted: 0"),
        "Second run should skip unchanged commands. stdout: {stdout}"
    );
}

#[test]
fn ci_extract_force_flag_bypasses_checks() {
    let dir = TempDir::new("ci_force");
    let output = TempDir::new("ci_force_out");
    let config_path = write_ci_config(&dir, &["echo"]);
    let manifest_path = dir.join("manifest.json");
    let bin = env!("CARGO_BIN_EXE_schema-discover");

    // First run
    let _ = std::process::Command::new(bin)
        .args([
            "ci-extract",
            "--config",
            config_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--output",
            output.path().to_str().unwrap(),
        ])
        .status()
        .expect("failed to run first pass");

    // Second run with --force
    let out = std::process::Command::new(bin)
        .args([
            "ci-extract",
            "--config",
            config_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--output",
            output.path().to_str().unwrap(),
            "--force",
        ])
        .output()
        .expect("failed to run forced pass");

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // With --force, it should not skip (Skipped: 0)
    assert!(
        stdout.contains("Skipped: 0") || stdout.contains("forced"),
        "Force flag should bypass version checks. stdout: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Migrate tests
// ---------------------------------------------------------------------------

#[test]
fn migrate_up_creates_tables() {
    let dir = TempDir::new("migrate_up");
    let db_path = dir.join("test.db");
    let bin = env!("CARGO_BIN_EXE_schema-discover");

    let status = std::process::Command::new(bin)
        .args([
            "migrate",
            "up",
            "--db",
            db_path.to_str().unwrap(),
            "--prefix",
            "cs_",
        ])
        .status()
        .expect("failed to run migrate up");

    assert!(status.success(), "migrate up should succeed");

    // Verify by running status
    let out = std::process::Command::new(bin)
        .args([
            "migrate",
            "status",
            "--db",
            db_path.to_str().unwrap(),
            "--prefix",
            "cs_",
        ])
        .output()
        .expect("failed to run migrate status");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Tables exist: yes"),
        "Tables should exist after up. stdout: {stdout}"
    );
}

#[test]
fn migrate_seed_populates_database() {
    let dir = TempDir::new("migrate_seed");
    let schema_dir = TempDir::new("migrate_seed_schemas");
    let db_path = dir.join("test.db");
    let bin = env!("CARGO_BIN_EXE_schema-discover");

    // Write test schemas
    write_test_schema(&schema_dir, "testcmd1");
    write_test_schema(&schema_dir, "testcmd2");

    // Run up first
    let _ = std::process::Command::new(bin)
        .args([
            "migrate",
            "up",
            "--db",
            db_path.to_str().unwrap(),
            "--prefix",
            "cs_",
        ])
        .status()
        .expect("failed to run migrate up");

    // Run seed
    let out = std::process::Command::new(bin)
        .args([
            "migrate",
            "seed",
            "--db",
            db_path.to_str().unwrap(),
            "--prefix",
            "cs_",
            "--source",
            schema_dir.path().to_str().unwrap(),
        ])
        .output()
        .expect("failed to run migrate seed");

    assert!(out.status.success(), "migrate seed should succeed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Commands inserted: 2"),
        "Should insert 2 commands. stdout: {stdout}"
    );
}

#[test]
fn migrate_refresh_clears_and_reseeds() {
    let dir = TempDir::new("migrate_refresh");
    let schema_dir = TempDir::new("migrate_refresh_schemas");
    let db_path = dir.join("test.db");
    let bin = env!("CARGO_BIN_EXE_schema-discover");

    write_test_schema(&schema_dir, "refreshcmd");

    // Run up + seed
    let _ = std::process::Command::new(bin)
        .args([
            "migrate",
            "up",
            "--db",
            db_path.to_str().unwrap(),
            "--prefix",
            "cs_",
        ])
        .status()
        .unwrap();

    let _ = std::process::Command::new(bin)
        .args([
            "migrate",
            "seed",
            "--db",
            db_path.to_str().unwrap(),
            "--prefix",
            "cs_",
            "--source",
            schema_dir.path().to_str().unwrap(),
        ])
        .status()
        .unwrap();

    // Now refresh with different schemas
    let schema_dir2 = TempDir::new("migrate_refresh_schemas2");
    write_test_schema(&schema_dir2, "newcmd1");
    write_test_schema(&schema_dir2, "newcmd2");
    write_test_schema(&schema_dir2, "newcmd3");

    let out = std::process::Command::new(bin)
        .args([
            "migrate",
            "refresh",
            "--db",
            db_path.to_str().unwrap(),
            "--prefix",
            "cs_",
            "--source",
            schema_dir2.path().to_str().unwrap(),
        ])
        .output()
        .expect("failed to run migrate refresh");

    assert!(out.status.success(), "migrate refresh should succeed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Commands inserted: 3"),
        "Should insert 3 new commands after refresh. stdout: {stdout}"
    );
}

#[test]
fn migrate_down_removes_tables() {
    let dir = TempDir::new("migrate_down");
    let db_path = dir.join("test.db");
    let bin = env!("CARGO_BIN_EXE_schema-discover");

    // Up first
    let _ = std::process::Command::new(bin)
        .args([
            "migrate",
            "up",
            "--db",
            db_path.to_str().unwrap(),
            "--prefix",
            "cs_",
        ])
        .status()
        .unwrap();

    // Down
    let status = std::process::Command::new(bin)
        .args([
            "migrate",
            "down",
            "--db",
            db_path.to_str().unwrap(),
            "--prefix",
            "cs_",
        ])
        .status()
        .expect("failed to run migrate down");

    assert!(status.success(), "migrate down should succeed");

    // Verify tables are gone
    let out = std::process::Command::new(bin)
        .args([
            "migrate",
            "status",
            "--db",
            db_path.to_str().unwrap(),
            "--prefix",
            "cs_",
        ])
        .output()
        .expect("failed to run migrate status");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Tables exist: no"),
        "Tables should not exist after down. stdout: {stdout}"
    );
}

#[test]
fn migrate_status_reports_correctly() {
    let dir = TempDir::new("migrate_status");
    let schema_dir = TempDir::new("migrate_status_schemas");
    let db_path = dir.join("test.db");
    let bin = env!("CARGO_BIN_EXE_schema-discover");

    write_test_schema(&schema_dir, "statuscmd");

    // Up + seed
    let _ = std::process::Command::new(bin)
        .args([
            "migrate",
            "up",
            "--db",
            db_path.to_str().unwrap(),
            "--prefix",
            "cs_",
        ])
        .status()
        .unwrap();

    let _ = std::process::Command::new(bin)
        .args([
            "migrate",
            "seed",
            "--db",
            db_path.to_str().unwrap(),
            "--prefix",
            "cs_",
            "--source",
            schema_dir.path().to_str().unwrap(),
        ])
        .status()
        .unwrap();

    // Check status
    let out = std::process::Command::new(bin)
        .args([
            "migrate",
            "status",
            "--db",
            db_path.to_str().unwrap(),
            "--prefix",
            "cs_",
        ])
        .output()
        .expect("failed to run migrate status");

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Tables exist: yes"));
    assert!(stdout.contains("Command count: 1"));
}
