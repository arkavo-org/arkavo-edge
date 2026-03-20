//! Integration tests for manifest persistence (save/load round-trip)

use arkavo_kv_cache::{ContextEntry, ContextManifest};
use arkavo_test_macros::spec;

fn sample_entry(name: &str, model_hash: &str) -> ContextEntry {
    ContextEntry {
        name: name.to_string(),
        bin_path: format!("/data/contexts/{name}.bin"),
        bin_hash: format!("{name}_hash"),
        model_id: "qwen3-0.6b-q8".to_string(),
        model_hash: model_hash.to_string(),
        built_at: "2026-02-22T12:00:00Z".to_string(),
        token_count: 256,
        trust_tier: String::new(),
    }
}

#[spec("KVC-004")]
#[test]
fn test_save_and_load_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = dir.path().join("manifest.json");
    let path_str = manifest_path.to_str().unwrap();

    let mut original = ContextManifest::new();
    original.register(sample_entry("agent-role", "model_v1"));
    original.register(sample_entry("user-prefs", "model_v1"));
    original.register(sample_entry("policy", "model_v1"));

    original.save(path_str).unwrap();
    let loaded = ContextManifest::load(path_str).unwrap();

    assert_eq!(loaded.version, 1);
    assert_eq!(loaded.len(), 3);
    assert_eq!(loaded.get("agent-role").unwrap().model_hash, "model_v1");
    assert_eq!(loaded.get("user-prefs").unwrap().token_count, 256);
    assert_eq!(loaded.get("policy").unwrap().model_id, "qwen3-0.6b-q8");
}

#[spec("KVC-004")]
#[test]
fn test_load_nonexistent_file() {
    let result = ContextManifest::load("/nonexistent/manifest.json");
    assert!(result.is_err());
}

#[spec("KVC-004")]
#[test]
fn test_save_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifest.json");
    let path_str = path.to_str().unwrap();

    let mut m1 = ContextManifest::new();
    m1.register(sample_entry("old", "v1"));
    m1.save(path_str).unwrap();

    let mut m2 = ContextManifest::new();
    m2.register(sample_entry("new", "v2"));
    m2.save(path_str).unwrap();

    let loaded = ContextManifest::load(path_str).unwrap();
    assert!(loaded.get("old").is_none());
    assert!(loaded.get("new").is_some());
}

#[spec("KVC-005")]
#[test]
fn test_needs_rebuild_after_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifest.json");
    let path_str = path.to_str().unwrap();

    let mut m = ContextManifest::new();
    m.register(sample_entry("role", "model_v1"));
    m.save(path_str).unwrap();

    let loaded = ContextManifest::load(path_str).unwrap();
    assert!(!loaded.needs_rebuild("role", "model_v1"));
    assert!(loaded.needs_rebuild("role", "model_v2"));
}
