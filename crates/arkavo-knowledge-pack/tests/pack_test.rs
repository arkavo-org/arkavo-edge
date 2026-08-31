//! KP-001 through KP-008: a pack built, verified, tampered with, and loaded.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use arkavo_crypto::AgentKeypair;
use arkavo_fingerprint::{IndexKey, NearDuplicateIndex, ReferenceIndex};
use arkavo_gguf_tdf::{
    Classification, ComponentRole, GgufTdfError, PayloadKeyUnwrapper, PayloadKeyWrapper,
    PreResolvedKey, WrappedKey,
};
use arkavo_knowledge_pack::{
    Entitlements, Lineage, PackBuilder, PackIndexes, SelectionError, load_pack, seal_blob,
    select_adapters, verify_pack,
};
use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
use arkavo_test_macros::spec;
use opentdf::TdfManifest;

/// Stands in for the KAS at wrap time: records the key the caller generated so
/// the test can hand it back at open time. Nothing here evaluates policy —
/// that is the KAS's job, and the point of the indirection is that this test
/// exercises the same code path production does.
struct CapturingWrapper {
    captured: std::sync::Mutex<Option<[u8; 32]>>,
}

impl PayloadKeyWrapper for CapturingWrapper {
    fn wrap(&self, payload_key: &[u8; 32]) -> Result<WrappedKey, GgufTdfError> {
        *self.captured.lock().expect("lock") = Some(*payload_key);
        Ok(WrappedKey {
            kas_url: "https://kas.example".to_string(),
            kid: None,
            wrapped_key: "AA==".to_string(),
        })
    }
}

/// A KAS that refuses, for the path where entitlement is missing.
struct DenyingKas;

impl PayloadKeyUnwrapper for DenyingKas {
    fn unwrap_key(&self, _manifest: &TdfManifest) -> Result<[u8; 32], GgufTdfError> {
        Err(GgufTdfError::KasDenied("not entitled".to_string()))
    }
}

fn index_key() -> Arc<IndexKey> {
    Arc::new(IndexKey::derive(&[13u8; 32], "pack-tests").expect("derive"))
}

fn corpus_document() -> String {
    use std::fmt::Write as _;
    (0..140).fold(String::new(), |mut text, i: usize| {
        let _ = write!(text, "t{}n{} ", (i * 37) % 991, (i * 13) % 577);
        text
    })
}

fn thresholds() -> serde_json::Value {
    serde_json::json!({
        "detector_version": "sentinel-0.1",
        "taxonomy_version": "1.0.0",
        "thresholds": { "credentials": 0.8 }
    })
}

/// Write a sealed index component and return the key needed to open it.
fn write_index_component(dir: &Path, key: &IndexKey) -> [u8; 32] {
    let mut reference = ReferenceIndex::builder(key, "1.0.0");
    reference.add_document(
        key,
        &corpus_document(),
        DataCategory::Internal,
        SensitivityLevel::Confidential,
        "board-minutes",
    );
    let mut near = NearDuplicateIndex::builder(key, "1.0.0");
    near.add_document(
        key,
        &corpus_document(),
        arkavo_fingerprint::EntryMeta {
            category: DataCategory::Internal,
            sensitivity: SensitivityLevel::Confidential,
            source_family: "board-minutes".into(),
        },
    );
    let indexes = PackIndexes {
        reference: reference.build(),
        near: Some(near.build()),
    };

    let wrapper = CapturingWrapper {
        captured: std::sync::Mutex::new(None),
    };
    let blob = seal_blob(
        &serde_json::to_vec(&indexes).expect("serialize"),
        &wrapper,
        &["https://attr.arkavo.com/clearance/confidential".to_string()],
        "application/json",
    )
    .expect("seal");
    std::fs::write(
        dir.join("index.tdf"),
        serde_json::to_vec(&blob).expect("serialize"),
    )
    .expect("write");

    wrapper.captured.lock().expect("lock").expect("a key")
}

struct Fixture {
    dir: tempfile::TempDir,
    key: AgentKeypair,
    payload_key: [u8; 32],
}

/// A pack with a sentinel, an index, and two adapters at different levels.
fn build_pack() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let staging = dir.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging");

    let payload_key = write_index_component(&staging, &index_key());
    std::fs::write(staging.join("sentinel.gguf.tdf"), b"sentinel weights").expect("write");
    std::fs::write(staging.join("adapter-legal.gguf.tdf"), b"legal weights").expect("write");
    std::fs::write(staging.join("adapter-finance.gguf.tdf"), b"finance weights").expect("write");

    let mut builder = PackBuilder::new("pack-1", "1.0.0", "qwen3.5-0.8b")
        .with_corpus_digest("deadbeef")
        .with_thresholds(thresholds())
        .with_lineage(Lineage::Root);
    builder
        .add_component(
            &staging.join("index.tdf"),
            ComponentRole::Index,
            Some(Classification::Confidential),
        )
        .expect("index");
    builder
        .add_component(
            &staging.join("sentinel.gguf.tdf"),
            ComponentRole::Sentinel,
            Some(Classification::Internal),
        )
        .expect("sentinel");
    builder
        .add_component(
            &staging.join("adapter-legal.gguf.tdf"),
            ComponentRole::Adapter {
                compartment: "legal".into(),
            },
            Some(Classification::Confidential),
        )
        .expect("legal");
    builder
        .add_component(
            &staging.join("adapter-finance.gguf.tdf"),
            ComponentRole::Adapter {
                compartment: "finance".into(),
            },
            Some(Classification::Restricted),
        )
        .expect("finance");

    let key = AgentKeypair::generate();
    builder
        .build(&dir.path().join("pack"), &key)
        .expect("build");
    Fixture {
        dir,
        key,
        payload_key,
    }
}

fn pack_root(fixture: &Fixture) -> std::path::PathBuf {
    fixture.dir.path().join("pack")
}

/// KP-002, KP-003: a pack round-trips and its signature verifies against the
/// organization anchor.
#[spec("KP-003")]
#[test]
fn a_signed_pack_verifies_against_its_anchor() {
    let fixture = build_pack();

    let verified = verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key()))
        .expect("a freshly built pack verifies");

    assert_eq!(verified.manifest.pack_id, "pack-1");
    assert_eq!(verified.absent, Vec::<String>::new());
    assert_eq!(verified.present.len(), 4);
    assert_eq!(verified.manifest.lineage, Lineage::Root);
}

/// KP-003 edge case: the anchor cannot be resolved, so the pack is refused.
/// There is no offline trust-on-first-use fallback.
#[spec("KP-003")]
#[test]
fn a_pack_with_no_resolvable_anchor_is_refused() {
    let fixture = build_pack();

    let refused = verify_pack(&pack_root(&fixture), None);

    assert!(refused.is_err(), "a pack must not verify without an anchor");
}

/// KP-003: a signature from the wrong key does not verify.
#[spec("KP-003")]
#[test]
fn a_pack_signed_by_another_key_is_refused() {
    let fixture = build_pack();
    let impostor = AgentKeypair::generate();

    assert!(verify_pack(&pack_root(&fixture), Some(&impostor.public_key())).is_err());
}

/// KP-004: a component modified after signing is caught by its digest before
/// it is used.
#[spec("KP-004")]
#[test]
fn a_tampered_component_is_rejected_by_digest() {
    let fixture = build_pack();
    std::fs::write(
        pack_root(&fixture).join("sentinel.gguf.tdf"),
        b"substituted weights",
    )
    .expect("tamper");

    let refused = verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key()));

    let message = refused
        .expect_err("a tampered component must be caught")
        .to_string();
    assert!(message.contains("sentinel.gguf.tdf"), "{message}");
}

/// KP-004 edge case: a modified manifest fails on the signature first, before
/// any digest is even consulted.
#[spec("KP-004")]
#[test]
fn a_tampered_manifest_fails_on_the_signature_first() {
    let fixture = build_pack();
    let path = pack_root(&fixture).join("manifest.json");
    let text = std::fs::read_to_string(&path).expect("read");
    std::fs::write(&path, text.replace("pack-1", "pack-2")).expect("tamper");

    let message = verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key()))
        .expect_err("a tampered manifest must be caught")
        .to_string();

    assert!(message.contains("signature"), "{message}");
}

/// KP-005: an egress node holds the sentinel and the index and not the
/// adapters. The manifest still verifies and the node can say what it holds.
#[spec("KP-005")]
#[test]
fn a_partial_set_verifies_and_reports_what_is_held() {
    let fixture = build_pack();
    for adapter in ["adapter-legal.gguf.tdf", "adapter-finance.gguf.tdf"] {
        std::fs::remove_file(pack_root(&fixture).join(adapter)).expect("remove");
    }

    let verified = verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key()))
        .expect("a partial set is not a tampered set");

    assert_eq!(verified.present.len(), 2);
    assert_eq!(verified.absent.len(), 2);
    assert!(verified.inventory().contains("adapter-legal.gguf.tdf"));
    assert!(verified.holds("index.tdf"));
}

/// KP-006: the pack carries the high-water mark of its components.
#[spec("KP-006")]
#[test]
fn the_pack_ceiling_is_the_high_water_mark_of_its_components() {
    let fixture = build_pack();

    let verified =
        verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key())).expect("verify");

    assert_eq!(verified.manifest.ceiling(), Classification::Restricted);
}

/// KP-001 edge case: two adapters cannot claim one compartment.
#[spec("KP-001")]
#[test]
fn two_adapters_for_one_compartment_are_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.tdf"), b"a").expect("write");
    std::fs::write(dir.path().join("b.tdf"), b"b").expect("write");
    let mut builder = PackBuilder::new("pack-dup", "1.0.0", "tok");
    for file in ["a.tdf", "b.tdf"] {
        builder
            .add_component(
                &dir.path().join(file),
                ComponentRole::Adapter {
                    compartment: "legal".into(),
                },
                Some(Classification::Internal),
            )
            .expect("add");
    }

    let refused = builder.build(&dir.path().join("pack"), &AgentKeypair::generate());

    assert!(
        refused.is_err(),
        "a compartment resolves to exactly one adapter"
    );
}

/// KP-001 edge case: a pack with no adapters is still a valid pack.
#[spec("KP-001")]
#[test]
fn a_pack_with_no_adapters_is_valid() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("sentinel.tdf"), b"s").expect("write");
    let mut builder = PackBuilder::new("pack-egress", "1.0.0", "tok").with_thresholds(thresholds());
    builder
        .add_component(
            &dir.path().join("sentinel.tdf"),
            ComponentRole::Sentinel,
            Some(Classification::Internal),
        )
        .expect("add");
    let key = AgentKeypair::generate();

    builder
        .build(&dir.path().join("pack"), &key)
        .expect("build");

    assert!(verify_pack(&dir.path().join("pack"), Some(&key.public_key())).is_ok());
}

/// KP-007: only the adapters a session is entitled to are selected.
#[spec("KP-007")]
#[test]
fn selection_returns_only_entitled_adapters() {
    let fixture = build_pack();
    let verified =
        verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key())).expect("verify");
    let entitlements = Entitlements::new(Classification::Confidential).with_compartment("legal");

    let selection = select_adapters(&verified.manifest, &entitlements).expect("select");

    assert_eq!(selection.adapters, ["adapter-legal.gguf.tdf"]);
    assert_eq!(selection.ceiling, Classification::Confidential);
    assert!(selection.trace().contains("adapter-legal"));
}

/// KP-007 edge case: a session entitled to no adapter is served by the base
/// model rather than refused.
#[spec("KP-007")]
#[test]
fn a_session_entitled_to_nothing_gets_the_base_model() {
    let fixture = build_pack();
    let verified =
        verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key())).expect("verify");

    let selection = select_adapters(
        &verified.manifest,
        &Entitlements::new(Classification::Public),
    )
    .expect("select");

    assert!(selection.is_base_only());
    assert!(selection.trace().contains("base model"));
}

/// KP-007: stacking adapters from two levels is refused unless the session has
/// accepted the high-water ceiling.
#[spec("KP-007")]
#[test]
fn mixed_level_stacking_is_refused_until_the_ceiling_is_accepted() {
    let fixture = build_pack();
    let verified =
        verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key())).expect("verify");
    let both = Entitlements::new(Classification::Restricted)
        .with_compartment("legal")
        .with_compartment("finance");

    let refused = select_adapters(&verified.manifest, &both);

    assert!(matches!(refused, Err(SelectionError::MixedLevels { .. })));
    let accepted = select_adapters(&verified.manifest, &both.accepting_high_water())
        .expect("accepting the ceiling permits the stack");
    assert_eq!(accepted.adapters.len(), 2);
    assert_eq!(accepted.ceiling, Classification::Restricted);
}

/// KP-007: clearance alone is not enough; the compartment must be held too.
#[spec("KP-007")]
#[test]
fn clearance_without_the_compartment_selects_nothing() {
    let fixture = build_pack();
    let verified =
        verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key())).expect("verify");

    let selection = select_adapters(
        &verified.manifest,
        &Entitlements {
            clearance: Classification::Restricted,
            compartments: BTreeSet::new(),
            accepts_high_water: true,
        },
    )
    .expect("select");

    assert!(selection.is_base_only());
}

/// SENT-004, KP-003: the runtime reads thresholds out of the verified manifest
/// and builds a cascade from the pack's own indices.
#[spec("SENT-004")]
#[test]
fn the_loader_takes_its_thresholds_from_the_verified_manifest() {
    let fixture = build_pack();
    let verified =
        verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key())).expect("verify");

    let loaded = load_pack(
        &verified,
        Some(&index_key()),
        &PreResolvedKey::new(fixture.payload_key),
    )
    .expect("load");

    assert_eq!(loaded.calibration.detector_version, "sentinel-0.1");
    assert_eq!(loaded.calibration.taxonomy_version, "1.0.0");
    assert_eq!(loaded.ceiling, Classification::Restricted);
    // Pattern tier plus both index tiers, all provisioned from the pack.
    assert_eq!(loaded.cascade.tier_names().len(), 3);
}

/// KP-003: no key release without entitlement. A denying KAS means no index,
/// and the cascade runs on what is left rather than pretending.
#[spec("KP-003")]
#[test]
fn a_denied_key_yields_no_index_rather_than_plaintext() {
    let fixture = build_pack();
    let verified =
        verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key())).expect("verify");

    let refused = load_pack(&verified, Some(&index_key()), &DenyingKas);

    assert!(refused.is_err(), "a denied key must not yield an index");
}

/// The cascade a verified pack produces actually recognizes the corpus it was
/// built from. Without this the wiring above could be inert and still pass.
#[spec("KP-011")]
#[test]
fn the_loaded_cascade_recognizes_the_corpus_it_was_built_from() {
    let fixture = build_pack();
    let verified =
        verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key())).expect("verify");
    let loaded = load_pack(
        &verified,
        Some(&index_key()),
        &PreResolvedKey::new(fixture.payload_key),
    )
    .expect("load");

    let evidence = loaded.cascade.inspect_unbudgeted(&corpus_document());

    assert!(
        evidence.findings().next().is_some(),
        "the pack's own corpus must be recognized by the pack's own index"
    );
}

/// KP-002: the manifest binds the pack, and a detached signature over it is
/// what makes the binding worth anything.
///
/// Retargeted from `gguf-tdf`'s profile members: the signature belongs to the
/// container that binds a set of components, not to any one component's
/// archive, and adding a `.sig` member to `gguf-tdf/1` to satisfy a test would
/// have put it in the wrong place.
#[spec("KP-002")]
#[test]
fn a_pack_ships_a_detached_signature_over_its_manifest() {
    let fixture = build_pack();
    let root = pack_root(&fixture);

    assert!(root.join("manifest.json").is_file());
    assert!(root.join("manifest.sig").is_file());

    let verified = verify_pack(&root, Some(&fixture.key.public_key())).expect("verify");
    let manifest = &verified.manifest;
    assert_eq!(manifest.corpus_snapshot_digest, "deadbeef");
    assert_eq!(manifest.taxonomy_version, "1.0.0");
    assert_eq!(manifest.tokenizer, "qwen3.5-0.8b");
    assert!(!manifest.thresholds.is_null(), "thresholds are bound");
    assert!(
        manifest.components.iter().all(|c| !c.digest.is_empty()),
        "every component carries a digest"
    );
}

/// KP-002 edge case: a component with no digest is refused before signing. A
/// signature over a contradiction is worse than no signature.
#[spec("KP-002")]
#[test]
fn a_component_without_a_digest_is_refused_before_signing() {
    let fixture = build_pack();
    let verified =
        verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key())).expect("verify");
    let mut manifest = verified.manifest;
    manifest.components[0].digest.clear();

    let refused = manifest.check();

    assert!(refused.is_err());
}

/// KP-002: the signature covers the bytes as written, so a manifest that is
/// re-serialized rather than re-read is not the manifest that was signed.
#[spec("KP-002")]
#[test]
fn the_signature_covers_the_manifest_bytes_as_written() {
    let fixture = build_pack();
    let path = pack_root(&fixture).join("manifest.json");
    let on_disk = std::fs::read(&path).expect("read");

    let parsed = arkavo_knowledge_pack::PackManifest::from_bytes(&on_disk).expect("parse");

    assert_eq!(
        parsed.to_bytes(),
        on_disk,
        "a round trip must reproduce the signed bytes exactly"
    );
}

/// KP-003: the embedded policy is checked before a key is requested, so a
/// component that was not wrapped under the clearance its ceiling implies is
/// refused without a KAS round-trip.
#[spec("KP-003")]
#[test]
fn a_component_wrapped_under_the_wrong_policy_is_refused_before_any_key_request() {
    /// Fails the test if it is ever asked for a key: reaching this would mean
    /// the pre-flight check did not fire.
    struct NeverAsked;
    impl PayloadKeyUnwrapper for NeverAsked {
        fn unwrap_key(&self, _manifest: &TdfManifest) -> Result<[u8; 32], GgufTdfError> {
            panic!("a key was requested for a component whose policy did not match");
        }
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let staging = dir.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging");

    // Wrapped under no attributes at all, but recorded as Confidential.
    let wrapper = CapturingWrapper {
        captured: std::sync::Mutex::new(None),
    };
    let blob = seal_blob(b"{}", &wrapper, &[], "application/json").expect("seal");
    std::fs::write(
        staging.join("index.tdf"),
        serde_json::to_vec(&blob).expect("serialize"),
    )
    .expect("write");

    let mut builder = PackBuilder::new("mislabelled", "1.0.0", "tok").with_thresholds(thresholds());
    builder
        .add_component(
            &staging.join("index.tdf"),
            ComponentRole::Index,
            Some(Classification::Confidential),
        )
        .expect("add");
    let key = AgentKeypair::generate();
    let root = dir.path().join("pack");
    builder.build(&root, &key).expect("build");

    let verified = verify_pack(&root, Some(&key.public_key())).expect("verify");
    let refused = load_pack(&verified, Some(&index_key()), &NeverAsked);

    let message = match refused {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a component wrapped under nothing must not be opened"),
    };
    assert!(message.contains("policy"), "{message}");
}

/// KP-003: a component wrapped under a *higher* clearance than it claims is
/// over-protected, which is harmless. Refusing it would reject a legitimate
/// pack, so the check is "at least", not "exactly".
#[spec("KP-003")]
#[test]
fn an_over_protected_component_is_accepted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let staging = dir.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging");

    let key = index_key();
    let mut reference = ReferenceIndex::builder(&key, "1.0.0");
    reference.add_document(
        &key,
        &corpus_document(),
        DataCategory::Internal,
        SensitivityLevel::Internal,
        "notes",
    );
    let indexes = PackIndexes {
        reference: reference.build(),
        near: None,
    };
    let wrapper = CapturingWrapper {
        captured: std::sync::Mutex::new(None),
    };
    // Wrapped under restricted, recorded as internal.
    let blob = seal_blob(
        &serde_json::to_vec(&indexes).expect("serialize"),
        &wrapper,
        &["https://attr.arkavo.com/clearance/restricted".to_string()],
        "application/json",
    )
    .expect("seal");
    std::fs::write(
        staging.join("index.tdf"),
        serde_json::to_vec(&blob).expect("serialize"),
    )
    .expect("write");
    let payload_key = wrapper.captured.lock().expect("lock").expect("a key");

    let mut builder = PackBuilder::new("over", "1.0.0", "tok").with_thresholds(thresholds());
    builder
        .add_component(
            &staging.join("index.tdf"),
            ComponentRole::Index,
            Some(Classification::Internal),
        )
        .expect("add");
    let signing = AgentKeypair::generate();
    let root = dir.path().join("pack");
    builder.build(&root, &signing).expect("build");

    let verified = verify_pack(&root, Some(&signing.public_key())).expect("verify");
    let loaded = load_pack(
        &verified,
        Some(&index_key()),
        &PreResolvedKey::new(payload_key),
    );

    assert!(loaded.is_ok(), "over-protection must not be a refusal");
}

/// KP-004: the digest rule is about *use*, not about listing. A component
/// swapped after verification must not be loaded on the strength of a check
/// that no longer describes it.
#[spec("KP-004")]
#[test]
fn a_component_swapped_after_verification_is_refused_at_load() {
    let fixture = build_pack();
    let verified =
        verify_pack(&pack_root(&fixture), Some(&fixture.key.public_key())).expect("verify");

    // Verified, then swapped — the window a load-time check has to close.
    std::fs::write(pack_root(&fixture).join("index.tdf"), b"{}").expect("swap");

    let refused = load_pack(
        &verified,
        Some(&index_key()),
        &PreResolvedKey::new(fixture.payload_key),
    );

    let message = match refused {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a swapped component must not be loaded"),
    };
    assert!(message.contains("changed on disk"), "{message}");
}

/// KP-006: a recorded ceiling below the content it covers is a lie the policy
/// pre-check would faithfully enforce, so the content gets the last word.
#[spec("KP-006")]
#[test]
fn a_ceiling_below_the_content_it_covers_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let staging = dir.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging");

    let key = index_key();
    let mut reference = ReferenceIndex::builder(&key, "1.0.0");
    reference.add_document(
        &key,
        &corpus_document(),
        DataCategory::Credentials,
        SensitivityLevel::Restricted,
        "secrets",
    );
    let indexes = PackIndexes {
        reference: reference.build(),
        near: None,
    };
    let wrapper = CapturingWrapper {
        captured: std::sync::Mutex::new(None),
    };
    // Wrapped and recorded as internal; the entries inside are restricted.
    let blob = seal_blob(
        &serde_json::to_vec(&indexes).expect("serialize"),
        &wrapper,
        &["https://attr.arkavo.com/clearance/internal".to_string()],
        "application/json",
    )
    .expect("seal");
    std::fs::write(
        staging.join("index.tdf"),
        serde_json::to_vec(&blob).expect("serialize"),
    )
    .expect("write");
    let payload_key = wrapper.captured.lock().expect("lock").expect("a key");

    let mut builder = PackBuilder::new("understated", "1.0.0", "tok").with_thresholds(thresholds());
    builder
        .add_component(
            &staging.join("index.tdf"),
            ComponentRole::Index,
            Some(Classification::Internal),
        )
        .expect("add");
    let signing = AgentKeypair::generate();
    let root = dir.path().join("pack");
    builder.build(&root, &signing).expect("build");

    let verified = verify_pack(&root, Some(&signing.public_key())).expect("verify");
    let refused = load_pack(&verified, Some(&key), &PreResolvedKey::new(payload_key));

    let message = match refused {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a ceiling below its content must be refused"),
    };
    assert!(message.contains("classified"), "{message}");
}

/// A second index or sentinel is not additive: lookup is by role, so one would
/// be silently ignored, and which one would depend on manifest order.
#[spec("KP-001")]
#[test]
fn a_pack_with_two_indexes_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.tdf"), b"a").expect("write");
    std::fs::write(dir.path().join("b.tdf"), b"b").expect("write");
    let mut builder = PackBuilder::new("two-indexes", "1.0.0", "tok");
    for file in ["a.tdf", "b.tdf"] {
        builder
            .add_component(
                &dir.path().join(file),
                ComponentRole::Index,
                Some(Classification::Internal),
            )
            .expect("add");
    }

    let refused = builder.build(&dir.path().join("pack"), &AgentKeypair::generate());

    assert!(refused.is_err());
}
