//! Every kit shipped under `examples/` must be one `arkavo agent -c <kit>`
//! can start on the day the test runs.
//!
//! `arkavo-swarmkit`'s own sweep (`example_kits_validate.rs`) proves the
//! examples parse and hash correctly, but it cannot see the router's model
//! registry and does not look at the clock, so examples naming retired
//! models and examples past `kit.expires` shipped while it passed. This
//! sweep runs the same check as `arkavo kit validate`, which lives in this
//! crate because model resolution does.
//!
//! A declared `kit.id` must match the content. An empty one is accepted:
//! most examples are authored with `kit.id: ""` ("populated at publish"),
//! which the agent start path runs as it is.
//!
//! The expiry half reads the real clock on purpose: an example that has
//! expired is broken for whoever copies it, and this test failing is the
//! reminder to refresh `kit.created` / `kit.expires` and recompute `kit.id`.

use std::path::{Path, PathBuf};

use arkavo_cli::commands::kit::validate_kit_at;

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
fn every_example_kit_can_be_started_today() {
    // Canonical so a failure names `examples/<kit>/...` without `..` hops.
    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .expect("examples directory exists");
    let mut kit_files = Vec::new();
    find_kit_files(&examples_dir, &mut kit_files);
    kit_files.sort();

    // Guards against the walk silently matching nothing after a directory
    // reshuffle, which would make the test pass while checking no kits.
    assert!(
        kit_files.len() >= 8,
        "expected at least 8 example kits under {}, found {}: {:?}",
        examples_dir.display(),
        kit_files.len(),
        kit_files
    );

    let now = chrono::Utc::now();
    let mut failures = Vec::new();
    for path in &kit_files {
        match validate_kit_at(path, now) {
            Ok(report) => assert!(
                report.id_matches || report.kit_id.is_empty(),
                "{}: validate_kit_at accepted a declared kit.id that does not match",
                path.display()
            ),
            Err(err) => failures.push(err.to_string()),
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} example kits cannot be started:\n{}",
        failures.len(),
        kit_files.len(),
        failures.join("\n")
    );
}
