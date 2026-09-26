#![cfg(feature = "sentinel")]
//! Provisioning mutates process-global state, so this file holds exactly one
//! test: its binary is its own process under both `cargo test` and nextest.

mod common;

use arkavo_cli::sentinel_wiring::SentinelRuntime;
use arkavo_gguf_tdf::{Classification, PreResolvedKey};
use arkavo_protocol::data_classification::SensitivityLevel;

/// A runtime provisioned from a sealed pack with a semantic tier, the shape
/// `arkavo chat --pack` builds.
fn test_runtime() -> SentinelRuntime {
    let (_dir, verified, payload_key) = common::sealed_pack_with_semantic_index(
        common::semantic_thresholds(0.3, None),
        SensitivityLevel::Confidential,
        Classification::Confidential,
    );
    SentinelRuntime::from_pack(
        &verified,
        Some(&common::index_key()),
        &PreResolvedKey::new(payload_key),
        Some(common::embedder()),
    )
    .expect("provision from the pack")
}

#[test]
fn provisioning_twice_is_refused() {
    arkavo_cli::sentinel_wiring::install();
    arkavo_cli::sentinel_wiring::provision(test_runtime())
        .expect("the first pack provisions into our policy");
    let second = arkavo_cli::sentinel_wiring::provision(test_runtime());
    assert!(second.unwrap_err().contains("already provisioned"));
}
