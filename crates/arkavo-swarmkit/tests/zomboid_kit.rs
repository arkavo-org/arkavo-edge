//! Pins the shipped Zomboid Survival Kit so an edit cannot silently stale its
//! kit.id, and confirms it parses + validates with the expected role topology.

use arkavo_swarmkit::{kit_id_for, parse_yaml};
use std::path::PathBuf;

fn load_zomboid_kit() -> String {
    // tests/ is at CARGO_MANIFEST_DIR (crates/arkavo-swarmkit); the example lives
    // two levels up under examples/.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/zomboid-survival-kit/zomboid-survival-kit.swarmkit.yaml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

#[test]
fn zomboid_kit_parses_and_validates() {
    // parse_yaml runs validate() internally — success here means the kit is valid.
    let manifest = parse_yaml(&load_zomboid_kit()).expect("zomboid kit must parse + validate");
    assert_eq!(manifest.kit.name, "Zomboid Survival Kit");
    assert_eq!(manifest.roles.len(), 5);
    let ids: Vec<&str> = manifest.roles.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["survivor", "threat", "scavenger", "medic", "critic"]);
}

#[test]
fn zomboid_kit_id_is_not_stale() {
    let yaml = load_zomboid_kit();
    let manifest = parse_yaml(&yaml).expect("parses");
    let recomputed = kit_id_for(&manifest).expect("hash computes");
    assert_eq!(
        manifest.kit.id, recomputed,
        "examples/zomboid-survival-kit kit.id is stale — re-run validate_kit and update the kit.id field"
    );
}
