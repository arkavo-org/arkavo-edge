//! Sealed-pack fixtures shared by the sentinel integration tests.
//!
//! Every item here must be reachable from each test binary that declares
//! `mod common;` — an unused helper is a dead-code warning, and the repo
//! forbids silencing those. Variation is therefore a parameter, not a second
//! helper.
//!
//! `arkavo-fingerprint`'s deterministic test embedder (`embed::tests::BagOfWords`)
//! is `#[cfg(test)]`-only and private to that crate, so this module keeps its
//! own stand-in — same trick (hash each word into a fixed bucket) so two texts
//! sharing words share direction, without pulling in a real model.

// `pub(crate)` here trips clippy's `redundant_pub_crate` and plain `pub` trips
// rustc's `unreachable_pub`; the test crates in this directory settle it the
// same way, as a module shared by test binaries is never reachable outside.
#![allow(unreachable_pub)]

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arkavo_crypto::AgentKeypair;
use arkavo_fingerprint::{
    Embedder, EmbedderRecord, EmbeddingPooling, IndexKey, SemanticCalibration,
    SemanticIndexBuilder, label_key,
};
use arkavo_gguf_tdf::{Classification, ComponentRole, GgufTdfError, PayloadKeyWrapper, WrappedKey};
use arkavo_knowledge_pack::{PackBuilder, PackIndexes, VerifiedPack, seal_blob, verify_pack};
use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};

/// The corpus document the semantic index is built from. Not a secret — a
/// distinctive sentence, which is what a semantic tier recognizes.
pub const SEMANTIC_CANARY: &str =
    "the northwind acquisition closes in march with a hidden indemnity clause";

/// The digest the index records for its embedder, and the one
/// [`WordHashEmbedder`] reports when it is the matching embedder.
pub const EMBEDDER_DIGEST: &str = "test-digest";

/// Deterministic stand-in for a real embedder: hashes each word into one of
/// 64 buckets with a fixed-key hasher, so the same text always embeds to the
/// same vector across runs and processes.
pub struct WordHashEmbedder {
    pub digest: &'static str,
}

impl Embedder for WordHashEmbedder {
    fn digest(&self) -> &str {
        self.digest
    }

    fn pooling(&self) -> EmbeddingPooling {
        EmbeddingPooling::Last
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; 64];
                for w in t.split_whitespace() {
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    w.to_lowercase().hash(&mut hasher);
                    v[(hasher.finish() % 64) as usize] += 1.0;
                }
                v
            })
            .collect())
    }
}

/// The embedder the index was built with.
pub fn embedder() -> Arc<dyn Embedder> {
    Arc::new(WordHashEmbedder {
        digest: EMBEDDER_DIGEST,
    })
}

/// The tenant key the fixture's reference index is keyed under.
pub fn index_key() -> Arc<IndexKey> {
    Arc::new(IndexKey::derive(&[31u8; 32], "semantic-load").expect("derive"))
}

fn embedder_record() -> EmbedderRecord {
    EmbedderRecord {
        source: "org/model/file.gguf".to_string(),
        sha256: EMBEDDER_DIGEST.to_string(),
        pooling: EmbeddingPooling::Last,
    }
}

/// Manifest thresholds in the object form: the semantic tier calibrated at
/// `margin` for the canary's label, plus the sentinel tier's table when one is
/// given.
///
/// The margin is a parameter because it decides what the fixture is for. A
/// margin of `-1.0` fires on any text at all, which suits a suite about
/// whether `load_pack` builds the tier; a pipeline test that must also release
/// clean text needs a margin a short unrelated span stays below.
pub fn semantic_thresholds(margin: f32, sentinel: Option<serde_json::Value>) -> serde_json::Value {
    let calibration = SemanticCalibration {
        detector_version: SemanticCalibration::detector_version_for(EMBEDDER_DIGEST),
        taxonomy_version: "1.0.0".to_string(),
        margins: BTreeMap::from([(
            label_key(DataCategory::Internal, SensitivityLevel::Confidential),
            margin,
        )]),
        embedder: embedder_record(),
    };
    let mut thresholds = serde_json::json!({
        "semantic": serde_json::to_value(calibration).expect("serialize calibration"),
    });
    if let Some(table) = sentinel {
        thresholds["sentinel"] = table;
    }
    thresholds
}

/// The wrap attribute a `recorded_ceiling` needs, in the taxonomy's own
/// vocabulary (`schemas/taxonomy-map.v1.json`'s clearance `order`), so a
/// fixture can wrap a component under exactly the clearance it will record —
/// satisfying `policy_covers_ceiling` regardless of what the test wants the
/// *content* check to do afterward.
fn clearance_attribute(ceiling: Classification) -> String {
    let level = match ceiling {
        Classification::Public => "public",
        Classification::Internal => "internal",
        Classification::Confidential => "confidential",
        Classification::Restricted => "restricted",
    };
    format!("https://attr.arkavo.com/clearance/{level}")
}

/// A pack whose index carries an empty reference section, an empty
/// near-duplicate section and a semantic section over `SEMANTIC_CANARY` at
/// `document_sensitivity`, sealed with the given manifest thresholds and
/// recorded (and wrapped) at `recorded_ceiling`. Returns the temp directory
/// (kept alive so `load_pack` can still read the pack from disk), the verified
/// pack, and the payload key the index was wrapped under.
pub fn sealed_pack_with_semantic_index(
    thresholds: serde_json::Value,
    document_sensitivity: SensitivityLevel,
    recorded_ceiling: Classification,
) -> (tempfile::TempDir, VerifiedPack, [u8; 32]) {
    struct Capturing(std::sync::Mutex<Option<[u8; 32]>>);
    impl PayloadKeyWrapper for Capturing {
        fn wrap(&self, payload_key: &[u8; 32]) -> Result<WrappedKey, GgufTdfError> {
            *self.0.lock().expect("lock") = Some(*payload_key);
            Ok(WrappedKey {
                kas_url: "https://kas.example".into(),
                kid: None,
                wrapped_key: "AA==".into(),
            })
        }
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let staging = dir.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging");

    let matching = WordHashEmbedder {
        digest: EMBEDDER_DIGEST,
    };
    let mut builder = SemanticIndexBuilder::new("1.0.0", embedder_record());
    builder
        .add_document(
            &matching,
            SEMANTIC_CANARY,
            DataCategory::Internal,
            document_sensitivity,
            "board-minutes",
        )
        .expect("add document");
    builder
        .add_anchor(
            &matching,
            "oxycodone prescribing information warns of misuse",
        )
        .expect("add anchor");
    let semantic_index = builder.build().expect("build semantic index");

    let key = index_key();
    // An empty near-duplicate index still joins the cascade, and it reports
    // every short span out of scope — the condition that once held every short
    // completion of a live pack forever. `pack index` writes one, so the
    // fixture carries one too.
    let indexes = PackIndexes {
        reference: arkavo_fingerprint::ReferenceIndex::builder(&key, "1.0.0").build(),
        near: Some(arkavo_fingerprint::NearDuplicateIndex::builder(&key, "1.0.0").build()),
        semantic: Some(semantic_index),
    };

    let wrapper = Capturing(std::sync::Mutex::new(None));
    let blob = seal_blob(
        &serde_json::to_vec(&indexes).expect("serialize"),
        &wrapper,
        &[clearance_attribute(recorded_ceiling)],
        "application/json",
    )
    .expect("seal");
    std::fs::write(
        staging.join("index.tdf"),
        serde_json::to_vec(&blob).expect("serialize"),
    )
    .expect("write");
    let payload_key = wrapper.0.lock().expect("lock").expect("a key");

    let mut pack_builder =
        PackBuilder::new("semantic-pack", "1.0.0", "qwen3.5-0.8b").with_thresholds(thresholds);
    pack_builder
        .add_component(
            &staging.join("index.tdf"),
            ComponentRole::Index,
            Some(recorded_ceiling),
        )
        .expect("component");
    let signing = AgentKeypair::generate();
    let root = dir.path().join("pack");
    pack_builder.build(&root, &signing).expect("build");

    let verified = verify_pack(&root, Some(&signing.public_key())).expect("verify");
    (dir, verified, payload_key)
}
