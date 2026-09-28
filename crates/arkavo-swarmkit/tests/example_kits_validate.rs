//! Durable gate over every SwarmKit kit under `examples/`. The per-kit tests
//! (e.g. `github_ops_kit_validates.rs`) cover shipped kits individually with
//! kit-specific role/tool assertions; this test instead sweeps the whole
//! `examples/` tree so newly converted kits are covered automatically
//! without anyone remembering to add a dedicated test file for each one.
//!
//! Two checks per kit:
//! - `arkavo_swarmkit::parse_yaml` succeeds (parse includes cross-block
//!   `validate`).
//! - The declared `kit.id` matches a fresh `kit_id_for` recomputation, so a
//!   kit edited without recomputing its BLAKE3 id is caught. Kits that
//!   author `kit.id` as the empty string (not yet published — see
//!   `Manifest::compute_kit_id`) skip this half of the check, mirroring the
//!   library's own `validate()` semantics.

use arkavo_swarmkit::{kit_id_for, parse_yaml};
use std::path::{Path, PathBuf};

/// Hand-rolled recursive walk (no new deps) collecting every
/// `*.swarmkit.yaml` / `*.swarmkit.yml` file under `dir`.
fn find_kit_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_kit_files(&path, out);
            continue;
        }
        let is_kit_file = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| {
                name.ends_with(".swarmkit.yaml") || name.ends_with(".swarmkit.yml")
            });
        if is_kit_file {
            out.push(path);
        }
    }
}

#[test]
fn all_example_kits_parse_validate_and_round_trip_id() {
    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut kit_files = Vec::new();
    find_kit_files(&examples_dir, &mut kit_files);

    // Guards against the glob silently matching nothing (e.g. a bad relative
    // path after a directory reshuffle).
    assert!(
        kit_files.len() >= 8,
        "expected at least 8 example kits under {}, found {}: {:?}",
        examples_dir.display(),
        kit_files.len(),
        kit_files
    );

    let mut failures = Vec::new();
    for path in &kit_files {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                failures.push(format!("{}: failed to read file: {e}", path.display()));
                continue;
            }
        };
        let manifest = match parse_yaml(&content) {
            Ok(m) => m,
            Err(e) => {
                failures.push(format!("{}: parse_yaml failed: {e}", path.display()));
                continue;
            }
        };
        if manifest.kit.id.is_empty() {
            continue;
        }
        match kit_id_for(&manifest) {
            Ok(expected) if expected == manifest.kit.id => {}
            Ok(expected) => failures.push(format!(
                "{}: kit.id {:?} does not match recomputed id {:?}",
                path.display(),
                manifest.kit.id,
                expected
            )),
            Err(e) => failures.push(format!(
                "{}: kit_id_for recomputation failed: {e}",
                path.display()
            )),
        }
    }

    assert!(
        failures.is_empty(),
        "example kit validation failures:\n{}",
        failures.join("\n")
    );
}
