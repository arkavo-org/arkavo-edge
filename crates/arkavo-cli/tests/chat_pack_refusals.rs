#![cfg(feature = "sentinel")]
// `#[tokio::test]` expands to `Runtime::block_on` on the test thread, outside
// any runtime — the case the lint exists to keep out of library code.
#![allow(clippy::disallowed_methods)]
//! `chat --pack` refusals that must happen before the embedder is fetched.
//!
//! Fetching the embedder is a download of hundreds of megabytes, so anything
//! that can fail from local state alone — a pack that cannot serve the index
//! the operator supplied keys for, or a key file that is missing or the wrong
//! size — is checked first. Each fixture names an embedder source the fetch
//! step refuses to parse, so a check that ran after the fetch would surface
//! the fetch's error instead of its own.
//!
//! Every test here fails before provisioning, so none touches the
//! process-global release policy and they can share one binary.

use std::path::PathBuf;

use arkavo_cli::commands::chat_pack::{PackArgs, provision_from_pack};
use arkavo_crypto::AgentKeypair;
use arkavo_gguf_tdf::{Classification, ComponentRole};
use arkavo_knowledge_pack::PackBuilder;

/// Semantic thresholds whose embedder source has no `<owner>/<repo>/<file>`
/// shape, so reaching the fetch fails at once, offline.
fn thresholds_naming_an_unfetchable_embedder() -> serde_json::Value {
    serde_json::json!({
        "semantic": {
            "detector_version": "semantic-test",
            "taxonomy_version": "1.0.0",
            "margins": { "internal:confidential": 0.3 },
            "embedder": {
                "source": "not-a-hub-path",
                "sha256": "test-digest",
                "pooling": "last"
            }
        }
    })
}

struct Fixture {
    _dir: tempfile::TempDir,
    args: PackArgs,
    pack: PathBuf,
}

/// A signed pack with one component of `role`, plus an anchor and well-formed
/// key files beside it.
fn fixture(role: ComponentRole) -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let staging = dir.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging");
    let component = staging.join("component.tdf");
    std::fs::write(&component, b"sealed component bytes").expect("component");

    let mut builder = PackBuilder::new("refusals", "1.0.0", "qwen3.5-0.8b")
        .with_thresholds(thresholds_naming_an_unfetchable_embedder());
    builder
        .add_component(&component, role, Some(Classification::Confidential))
        .expect("add component");
    let signing = AgentKeypair::generate();
    let pack = dir.path().join("pack");
    builder.build(&pack, &signing).expect("build");

    let anchor = dir.path().join("org.pub");
    std::fs::write(&anchor, signing.public_key().to_bytes()).expect("anchor");
    let index_key = dir.path().join("tenant.key");
    std::fs::write(&index_key, [9u8; 32]).expect("index key");
    let payload_key = dir.path().join("payload.key");
    std::fs::write(&payload_key, [7u8; 32]).expect("payload key");

    Fixture {
        args: PackArgs {
            pack: pack.clone(),
            anchor,
            index_key,
            index_id: "default".to_string(),
            payload_key,
        },
        pack,
        _dir: dir,
    }
}

async fn refusal(args: &PackArgs) -> String {
    match provision_from_pack(args).await {
        Ok(_) => panic!("the pack must be refused"),
        Err(e) => e,
    }
}

fn assert_not_the_fetch(err: &str) {
    assert!(
        !err.contains("embedder source"),
        "the embedder fetch ran first: {err}"
    );
}

#[tokio::test]
async fn a_pack_whose_index_is_not_held_here_is_refused() {
    let f = fixture(ComponentRole::Index);
    std::fs::remove_file(f.pack.join("component.tdf")).expect("drop the index");
    let err = refusal(&f.args).await;
    assert_not_the_fetch(&err);
    assert!(err.contains("component.tdf"), "{err}");
    assert!(err.contains("not held"), "{err}");
}

#[tokio::test]
async fn a_pack_with_no_index_component_is_refused() {
    let f = fixture(ComponentRole::Sentinel);
    let err = refusal(&f.args).await;
    assert_not_the_fetch(&err);
    assert!(err.contains("no index component"), "{err}");
}

#[tokio::test]
async fn a_missing_index_key_fails_before_the_embedder_fetch() {
    let mut f = fixture(ComponentRole::Index);
    f.args.index_key = f.args.index_key.with_file_name("absent.key");
    let err = refusal(&f.args).await;
    assert_not_the_fetch(&err);
    assert!(err.contains("tenant index key"), "{err}");
}

#[tokio::test]
async fn a_wrong_size_payload_key_fails_before_the_embedder_fetch() {
    let f = fixture(ComponentRole::Index);
    std::fs::write(&f.args.payload_key, [7u8; 31]).expect("short key");
    let err = refusal(&f.args).await;
    assert_not_the_fetch(&err);
    assert!(err.contains("exactly 32 bytes"), "{err}");
}
