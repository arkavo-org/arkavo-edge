//! Integration tests for `arkavo kit init` / `arkavo kit validate`.
//!
//! Exercises the pure `commands::kit` functions directly (no process spawn)
//! against a temp directory, per the Task 1 brief.

use arkavo_cli::commands::agent::deprecated_init;
use arkavo_cli::commands::kit::{init_kit, validate_kit};
use std::fs;

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("create tempdir")
}

#[test]
fn init_creates_valid_kit_with_single_role_and_skill() {
    let dir = tempdir();
    let report = init_kit(dir.path(), "demo-agent").expect("init_kit should succeed");

    assert!(report.path.exists(), "kit file should be written");

    let content = fs::read_to_string(&report.path).expect("read written kit");
    let manifest = arkavo_swarmkit::parse_yaml(&content).expect("written kit must parse+validate");

    let recomputed = arkavo_swarmkit::kit_id_for(&manifest).expect("recompute kit.id");
    assert_eq!(
        manifest.kit.id, recomputed,
        "declared kit.id must match recomputed hash"
    );
    assert_eq!(manifest.kit.id, report.kit_id);

    let runtime = manifest.runtime.expect("runtime block must be present");
    assert_eq!(runtime.local_dev, Some(true));

    assert_eq!(manifest.roles.len(), 1, "exactly one role expected");
    assert_eq!(
        manifest.roles[0].skills.len(),
        1,
        "exactly one inline skill expected"
    );
}

#[test]
fn init_errors_when_file_already_exists_and_does_not_modify_it() {
    let dir = tempdir();
    let first = init_kit(dir.path(), "dup-agent").expect("first init should succeed");
    let original = fs::read_to_string(&first.path).unwrap();

    let err = init_kit(dir.path(), "dup-agent");
    assert!(err.is_err(), "second init with same name must fail");

    let after = fs::read_to_string(&first.path).unwrap();
    assert_eq!(original, after, "existing kit file must not be modified");
}

#[test]
fn discover_kit_path_finds_generated_kit() {
    let dir = tempdir();
    init_kit(dir.path(), "discoverable-agent").expect("init_kit should succeed");

    let found = arkavo_swarmkit::discover_kit_path(dir.path()).expect("discovery should find kit");
    assert!(
        found
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".swarmkit.yaml")
    );
}

#[test]
fn validate_succeeds_on_generated_kit() {
    let dir = tempdir();
    let report = init_kit(dir.path(), "validated-agent").expect("init_kit should succeed");

    let result = validate_kit(&report.path).expect("validate_kit should succeed on a fresh kit");
    assert_eq!(result.kit_name, "validated-agent");
    assert_eq!(result.kit_id, report.kit_id);
    assert!(result.id_matches);
}

#[test]
fn validate_fails_on_tampered_kit_id() {
    let dir = tempdir();
    let report = init_kit(dir.path(), "tampered-agent").expect("init_kit should succeed");

    let content = fs::read_to_string(&report.path).unwrap();
    let tampered = content.replace(&report.kit_id, "blake3:tampered-hash-value-not-real");
    fs::write(&report.path, tampered).unwrap();

    let result = validate_kit(&report.path);
    assert!(result.is_err(), "tampered kit.id must fail validation");
}

#[test]
fn validate_fails_on_invalid_yaml() {
    let dir = tempdir();
    let path = dir.path().join("broken.swarmkit.yaml");
    fs::write(&path, "not: [valid, yaml: structure").unwrap();

    let result = validate_kit(&path);
    assert!(result.is_err(), "invalid YAML must fail validation");
}

/// Regression for finding 3: `kit init` must reject a name that could
/// escape `.arkavo/` rather than writing outside it.
#[test]
fn init_rejects_name_with_path_separator_and_writes_nothing() {
    let dir = tempdir();

    let result = init_kit(dir.path(), "../evil");
    assert!(result.is_err(), "a name containing '/' must be rejected");

    // Nothing should have been written anywhere under or above the tempdir.
    let arkavo_dir = dir.path().join(".arkavo");
    if arkavo_dir.exists() {
        let entries: Vec<_> = fs::read_dir(&arkavo_dir).unwrap().collect();
        assert!(
            entries.is_empty(),
            "no file should have been written into .arkavo/"
        );
    }
    assert!(
        !dir.path()
            .parent()
            .unwrap()
            .join("evil.swarmkit.yaml")
            .exists(),
        "nothing should have been written outside the tempdir"
    );
}

/// Companion to the above: a bare `..` name must also be rejected.
#[test]
fn init_rejects_dotdot_name_and_writes_nothing() {
    let dir = tempdir();

    let result = init_kit(dir.path(), "..");
    assert!(result.is_err(), "a name of '..' must be rejected");

    let arkavo_dir = dir.path().join(".arkavo");
    if arkavo_dir.exists() {
        let entries: Vec<_> = fs::read_dir(&arkavo_dir).unwrap().collect();
        assert!(
            entries.is_empty(),
            "no file should have been written into .arkavo/"
        );
    }
}

/// `arkavo agent init` is a deprecated alias of `kit init` (Phase S4). It must
/// no longer write AGENTS.md at all — it delegates entirely to the same
/// manifest writer `kit init` uses, so the two commands can never drift.
#[test]
fn agent_init_deprecated_alias_writes_swarmkit_manifest_not_agents_md() {
    let dir = tempdir();
    let report =
        deprecated_init(dir.path(), "legacy-agent").expect("deprecated_init should succeed");

    assert!(
        report.path.exists(),
        "deprecated agent init must still write a manifest file"
    );
    assert!(
        report
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".swarmkit.yaml"),
        "deprecated agent init must write a .swarmkit.yaml file, got {:?}",
        report.path
    );
    assert!(
        !dir.path().join(".arkavo").join("AGENTS.md").exists(),
        "deprecated agent init must not write AGENTS.md"
    );

    let validated = validate_kit(&report.path)
        .expect("manifest written by deprecated agent init must validate");
    assert_eq!(validated.kit_name, "legacy-agent");
    assert_eq!(validated.kit_id, report.kit_id);
    assert!(validated.id_matches);
}
