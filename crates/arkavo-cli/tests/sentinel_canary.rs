//! Phase 4 acceptance: a seeded canary in a completion is caught before release.
//!
//! End to end over the path that matters. The mock provider produces a
//! completion carrying corpus text; that completion is streamed chunk by chunk
//! through the release gate; the gate holds each window until the cascade has
//! inspected it. The assertion is not that the cascade *found* the canary — a
//! unit test does that — but that the consumer never saw it, which is the only
//! property that survives the fact that a completion cannot be unstreamed.

#![cfg(feature = "sentinel")]
#![allow(clippy::disallowed_methods)]

use std::sync::Arc;

use arkavo_cli::mock_provider::{MockProvider, MockProviderConfig};
use arkavo_cli::sentinel_wiring::{CascadeGate, CascadeSource};
use arkavo_critic::{ClassificationSource, SentinelCheck};
use arkavo_fingerprint::{IndexKey, ReferenceIndex, ReferenceTier};
use arkavo_llm::{GATE_BLOCKED, ReleaseGate, StreamResponse, gated};
use arkavo_protocol::RegexInferencer;
use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
use arkavo_sentinel::{Cascade, CascadeTier, PatternTier};
use arkavo_test_macros::spec;
use futures::StreamExt;

/// The corpus document the index is built from. Not a secret — a distinctive
/// sentence, which is what a reference tier recognizes.
const CANARY: &str = "the northwind acquisition closes in the third quarter pending board approval";

fn cascade_with_canary() -> Arc<Cascade> {
    let key = Arc::new(IndexKey::derive(&[11u8; 32], "canary-corpus").expect("derive"));
    let mut builder = ReferenceIndex::builder(&key, "1.0.0");
    builder.add_document(
        &key,
        CANARY,
        DataCategory::Internal,
        SensitivityLevel::Confidential,
        "board-minutes",
    );
    let index = Arc::new(builder.build());
    Arc::new(
        Cascade::new("1.0.0")
            .with_tier(Arc::new(PatternTier::new(Arc::new(RegexInferencer::new()))))
            .with_tier(Arc::new(ReferenceTier::loaded(index, key)) as Arc<dyn CascadeTier>),
    )
}

/// Chunk text the way a model streams it, so the canary straddles boundaries.
fn stream_of(text: &str) -> Vec<arkavo_llm::Result<StreamResponse>> {
    let mut chunks: Vec<arkavo_llm::Result<StreamResponse>> = text
        .as_bytes()
        .chunks(7)
        .map(|c| {
            Ok(StreamResponse {
                content: String::from_utf8_lossy(c).to_string(),
                reasoning_content: None,
                done: false,
                inference_timing: None,
                ..Default::default()
            })
        })
        .collect();
    chunks.push(Ok(StreamResponse {
        content: String::new(),
        reasoning_content: None,
        done: true,
        inference_timing: None,
        ..Default::default()
    }));
    chunks
}

async fn completion_containing(text: &str) -> String {
    // Set once: these tests run in parallel and `set_var` is not thread-safe
    // against a concurrent read.
    static MOCK: std::sync::Once = std::sync::Once::new();
    MOCK.call_once(|| {
        // SAFETY: inside `Once`, before any test reads the variable.
        unsafe { std::env::set_var("ARKAVO_MOCK_PROVIDER", "1") };
    });
    assert!(MockProvider::is_enabled());

    // No key validation here: this test is about what leaves on the way out,
    // and an auth failure would answer a different question.
    let mut config = MockProviderConfig {
        validate_api_key: false,
        response_delay_ms: 0,
        ..Default::default()
    };
    config
        .custom_responses
        .insert("summarize".to_string(), text.to_string());
    let provider = MockProvider::with_config(config);

    provider
        .chat_completion("test-key", "mock", "summarize the board minutes")
        .await
        .expect("the mock provider answers")
        .content
}

/// SENT-007: the canary never reaches the consumer.
#[spec("SENT-007")]
#[tokio::test]
async fn a_seeded_canary_in_a_completion_is_caught_before_release() {
    let completion =
        completion_containing(&format!("Here is the summary. {CANARY}. Regards.")).await;
    assert!(completion.contains("northwind"), "the mock must produce it");

    let gate: Arc<dyn ReleaseGate> = Arc::new(CascadeGate::new(
        cascade_with_canary(),
        SensitivityLevel::Internal,
    ));
    let mut stream = gated(
        Box::pin(futures::stream::iter(stream_of(&completion))),
        gate,
    );

    let mut seen = String::new();
    let mut refusal = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => seen.push_str(&chunk.content),
            Err(e) => {
                refusal = Some(e.to_string());
                break;
            }
        }
    }

    assert!(refusal.is_some(), "the stream must be cut, not completed");
    assert!(
        !seen.contains("northwind"),
        "the canary reached the consumer: {seen:?}"
    );
}

/// SENT-011: the consumer is told nothing about why. A message naming the label
/// or the position would let a caller bisect what it could not see.
#[spec("SENT-011")]
#[tokio::test]
async fn the_refusal_tells_the_consumer_nothing_about_the_finding() {
    let completion = completion_containing(&format!("Summary: {CANARY}.")).await;
    let gate: Arc<dyn ReleaseGate> = Arc::new(CascadeGate::new(
        cascade_with_canary(),
        SensitivityLevel::Internal,
    ));
    let mut stream = gated(
        Box::pin(futures::stream::iter(stream_of(&completion))),
        gate,
    );

    let mut refusal = String::new();
    while let Some(item) = stream.next().await {
        if let Err(e) = item {
            refusal = e.to_string();
            break;
        }
    }

    assert!(refusal.contains(GATE_BLOCKED), "{refusal}");
    assert!(!refusal.contains("northwind"), "{refusal}");
    assert!(!refusal.contains("board-minutes"), "{refusal}");
    assert!(!refusal.contains("Confidential"), "{refusal}");
}

/// A completion with nothing in it still arrives whole. A gate that blocked
/// everything would pass the test above and be useless.
#[spec("SENT-007")]
#[tokio::test]
async fn an_unremarkable_completion_still_reaches_the_consumer() {
    let clean = "Here is a summary of the weather this week, which was mild and unremarkable \
                 throughout, with light rain on thursday and clear skies by the weekend.";
    let completion = completion_containing(clean).await;

    let gate: Arc<dyn ReleaseGate> = Arc::new(CascadeGate::new(
        cascade_with_canary(),
        SensitivityLevel::Internal,
    ));
    let mut stream = gated(
        Box::pin(futures::stream::iter(stream_of(&completion))),
        gate,
    );

    let mut seen = String::new();
    while let Some(item) = stream.next().await {
        seen.push_str(
            &item
                .expect("a clean completion must not be refused")
                .content,
        );
    }

    assert_eq!(seen, clean);
}

/// SENT-009: a model whose ceiling is Confidential streams nothing partial, so
/// a canary in the tail cannot have escaped in the head.
#[spec("SENT-009")]
#[tokio::test]
async fn a_confidential_model_releases_nothing_before_the_completion_is_whole() {
    let completion =
        completion_containing(&format!("A long and entirely ordinary preamble. {CANARY}.")).await;

    let gate: Arc<dyn ReleaseGate> = Arc::new(CascadeGate::new(
        cascade_with_canary(),
        SensitivityLevel::Confidential,
    ));
    let mut stream = gated(
        Box::pin(futures::stream::iter(stream_of(&completion))),
        gate,
    );

    let mut seen = String::new();
    let mut refused = false;
    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => seen.push_str(&chunk.content),
            Err(_) => {
                refused = true;
                break;
            }
        }
    }

    assert!(refused);
    assert!(
        seen.is_empty(),
        "not even the ordinary preamble may be streamed: {seen:?}"
    );
}

/// SENT-014: the same cascade, read through the critic pipeline, contributes
/// evidence rather than a verdict.
#[spec("SENT-014")]
#[tokio::test]
async fn the_critic_pipeline_receives_evidence_for_the_same_span() {
    let source = CascadeSource::new(cascade_with_canary());

    let evidence = source.inspect(CANARY);

    assert!(evidence.labels > 0, "the canary must be labelled");
    assert!(!evidence.has_gap);
    // And the check built on it never fails the pipeline.
    let check = SentinelCheck::new(Arc::new(CascadeSource::new(cascade_with_canary())));
    assert!(arkavo_critic::VerificationCheck::skip_after_failure(&check));
}

/// KP-003 through SENT-007, end to end: a pack is sealed, verified, loaded, and
/// the gate it provisions catches the pack's own corpus in a completion.
///
/// This is the phase's point. Phase 4 built the gate and nothing constructed
/// one; here the construction comes from a signed manifest, so what the gate
/// enforces is what somebody signed rather than what the local operator
/// configured.
#[spec("KP-003")]
#[tokio::test]
async fn a_verified_pack_provisions_a_gate_that_catches_its_own_corpus() {
    use arkavo_cli::sentinel_wiring::SentinelRuntime;
    use arkavo_crypto::AgentKeypair;
    use arkavo_gguf_tdf::{
        ComponentRole, GgufTdfError, PayloadKeyWrapper, PreResolvedKey, WrappedKey,
    };
    use arkavo_knowledge_pack::{PackBuilder, PackIndexes, seal_blob, verify_pack};

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

    // An index over the canary, wrapped as a component.
    let key = Arc::new(IndexKey::derive(&[21u8; 32], "e2e-pack").expect("derive"));
    let mut reference = ReferenceIndex::builder(&key, "1.0.0");
    reference.add_document(
        &key,
        CANARY,
        DataCategory::Internal,
        SensitivityLevel::Confidential,
        "board-minutes",
    );
    let indexes = PackIndexes {
        reference: reference.build(),
        near: None,
        semantic: None,
    };
    let wrapper = Capturing(std::sync::Mutex::new(None));
    // The entries are Confidential; wrap and record at that level. Anything
    // weaker is the lie the load-time ceiling check exists to catch.
    let blob = seal_blob(
        &serde_json::to_vec(&indexes).expect("serialize"),
        &wrapper,
        &["https://attr.arkavo.com/clearance/confidential".to_string()],
        "application/json",
    )
    .expect("seal");
    std::fs::write(
        staging.join("index.tdf"),
        serde_json::to_vec(&blob).expect("serialize"),
    )
    .expect("write");
    let payload_key = wrapper.0.lock().expect("lock").expect("a key");

    let mut builder =
        PackBuilder::new("e2e-pack", "1.0.0", "qwen3.5-0.8b").with_thresholds(serde_json::json!({
            "detector_version": "sentinel-0.1",
            "taxonomy_version": "1.0.0",
            "thresholds": { "credentials": 0.8 }
        }));
    builder
        .add_component(
            &staging.join("index.tdf"),
            ComponentRole::Index,
            Some(arkavo_gguf_tdf::Classification::Confidential),
        )
        .expect("component");
    let signing = AgentKeypair::generate();
    let root = dir.path().join("pack");
    builder.build(&root, &signing).expect("build");

    let verified = verify_pack(&root, Some(&signing.public_key())).expect("verify");
    let runtime = SentinelRuntime::from_pack(
        &verified,
        Some(&key),
        &PreResolvedKey::new(payload_key),
        None,
    )
    .expect("provision from the pack");

    // SENT-004: the thresholds came out of the signed manifest.
    assert_eq!(
        runtime
            .calibration
            .as_ref()
            .expect("sentinel table")
            .detector_version,
        "sentinel-0.1"
    );

    let completion = completion_containing(&format!("Summary. {CANARY}. Regards.")).await;
    let gate: Arc<dyn ReleaseGate> = Arc::new(runtime.gate());
    let mut stream = gated(
        Box::pin(futures::stream::iter(stream_of(&completion))),
        gate,
    );

    let mut seen = String::new();
    let mut refused = false;
    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => seen.push_str(&chunk.content),
            Err(_) => {
                refused = true;
                break;
            }
        }
    }

    assert!(refused, "the pack's own corpus must be caught");
    assert!(!seen.contains("northwind"), "{seen}");
}

/// Loading a pack whose index carries a semantic section.
///
/// `arkavo-fingerprint`'s deterministic test embedder (`embed::tests::BagOfWords`)
/// is `#[cfg(test)]`-only and private to that crate, so this module keeps its
/// own stand-in — same trick (hash each word into a fixed bucket) so two texts
/// sharing words share direction, without pulling in a real model.
mod semantic_load {
    use std::collections::BTreeMap;
    use std::hash::{Hash, Hasher};
    use std::sync::Arc;

    use arkavo_crypto::AgentKeypair;
    use arkavo_fingerprint::{
        Embedder, EmbedderRecord, EmbeddingPooling, IndexKey, SemanticCalibration,
        SemanticIndexBuilder, label_key,
    };
    use arkavo_gguf_tdf::{
        Classification, ComponentRole, GgufTdfError, PayloadKeyWrapper, PreResolvedKey, WrappedKey,
    };
    use arkavo_knowledge_pack::{
        LoadError, PackBuilder, PackIndexes, load_pack, seal_blob, verify_pack,
    };
    use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};

    const SEMANTIC_CANARY: &str =
        "the northwind acquisition closes in march with a hidden indemnity clause";
    const EMBEDDER_DIGEST: &str = "test-digest";
    const OTHER_DIGEST: &str = "some-other-digest";

    /// Deterministic stand-in for a real embedder: hashes each word into one of
    /// 64 buckets with a fixed-key hasher, so the same text always embeds to
    /// the same vector across runs and processes.
    struct WordHashEmbedder {
        digest: &'static str,
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

    fn embedder_record() -> EmbedderRecord {
        EmbedderRecord {
            source: "org/model/file.gguf".to_string(),
            sha256: EMBEDDER_DIGEST.to_string(),
            pooling: EmbeddingPooling::Last,
        }
    }

    fn semantic_calibration() -> SemanticCalibration {
        let mut margins = BTreeMap::new();
        // A margin threshold low enough that any embedding of the canary
        // clears it: this suite is about whether `load_pack` builds the tier
        // at all, not about the tier's own judgement, which is covered in
        // `arkavo-fingerprint`.
        margins.insert(
            label_key(DataCategory::Internal, SensitivityLevel::Confidential),
            -1.0,
        );
        SemanticCalibration {
            detector_version: SemanticCalibration::detector_version_for(EMBEDDER_DIGEST),
            taxonomy_version: "1.0.0".to_string(),
            margins,
            embedder: embedder_record(),
        }
    }

    /// The manifest thresholds `sealed_pack_with_semantic_index` normally
    /// carries: only the semantic tier's calibration.
    fn thresholds_with_semantic() -> serde_json::Value {
        serde_json::json!({
            "semantic": serde_json::to_value(semantic_calibration()).expect("serialize calibration"),
        })
    }

    /// The wrap attribute a `recorded_ceiling` needs, in the taxonomy's own
    /// vocabulary (`schemas/taxonomy-map.v1.json`'s clearance `order`), so a
    /// fixture can wrap a component under exactly the clearance it will
    /// record — satisfying `policy_covers_ceiling` regardless of what the
    /// test wants the *content* check to do afterward.
    fn clearance_attribute(ceiling: Classification) -> String {
        let level = match ceiling {
            Classification::Public => "public",
            Classification::Internal => "internal",
            Classification::Confidential => "confidential",
            Classification::Restricted => "restricted",
        };
        format!("https://attr.arkavo.com/clearance/{level}")
    }

    /// A pack whose index carries a semantic section over `SEMANTIC_CANARY`
    /// at `document_sensitivity`, sealed with the given manifest thresholds
    /// and recorded (and wrapped) at `recorded_ceiling`. Returns the temp
    /// directory (kept alive so `load_pack` can still read the pack from
    /// disk), the verified pack, and the payload key the index was wrapped
    /// under.
    fn sealed_pack_with_semantic_index(
        thresholds: serde_json::Value,
        document_sensitivity: SensitivityLevel,
        recorded_ceiling: Classification,
    ) -> (
        tempfile::TempDir,
        arkavo_knowledge_pack::VerifiedPack,
        [u8; 32],
    ) {
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

        let mut builder = SemanticIndexBuilder::new("1.0.0", embedder_record());
        builder
            .add_document(
                &WordHashEmbedder {
                    digest: EMBEDDER_DIGEST,
                },
                SEMANTIC_CANARY,
                DataCategory::Internal,
                document_sensitivity,
                "board-minutes",
            )
            .expect("add document");
        builder
            .add_anchor(
                &WordHashEmbedder {
                    digest: EMBEDDER_DIGEST,
                },
                "oxycodone prescribing information warns of misuse",
            )
            .expect("add anchor");
        let semantic_index = builder.build().expect("build semantic index");

        let indexes = PackIndexes {
            reference: {
                let key = Arc::new(IndexKey::derive(&[31u8; 32], "semantic-load").expect("derive"));
                arkavo_fingerprint::ReferenceIndex::builder(&key, "1.0.0").build()
            },
            near: None,
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

    /// SENT-013: a declared semantic requirement is refused, not dropped,
    /// when no embedder is provisioned to score it.
    #[test]
    fn a_semantic_index_without_an_embedder_is_refused() {
        let (_dir, verified, payload_key) = sealed_pack_with_semantic_index(
            thresholds_with_semantic(),
            SensitivityLevel::Confidential,
            Classification::Confidential,
        );
        let key = Arc::new(IndexKey::derive(&[31u8; 32], "semantic-load").expect("derive"));

        let refused = load_pack(
            &verified,
            Some(&key),
            &PreResolvedKey::new(payload_key),
            None,
        );

        assert!(matches!(refused, Err(LoadError::EmbedderMissing)));
    }

    /// `SemanticTier::new` re-checks the index's recorded digest against the
    /// embedder actually provisioned; `load_pack` must surface that refusal
    /// through `LoadError::Semantic` rather than panicking or swallowing it.
    #[test]
    fn a_mismatched_embedder_is_refused_as_a_semantic_load_error() {
        let (_dir, verified, payload_key) = sealed_pack_with_semantic_index(
            thresholds_with_semantic(),
            SensitivityLevel::Confidential,
            Classification::Confidential,
        );
        let key = Arc::new(IndexKey::derive(&[31u8; 32], "semantic-load").expect("derive"));
        let wrong_embedder: Arc<dyn Embedder> = Arc::new(WordHashEmbedder {
            digest: OTHER_DIGEST,
        });

        let refused = load_pack(
            &verified,
            Some(&key),
            &PreResolvedKey::new(payload_key),
            Some(wrong_embedder),
        );

        assert!(matches!(refused, Err(LoadError::Semantic(_))));
    }

    /// With a matching embedder and calibration, the semantic tier joins the
    /// cascade like any other.
    #[test]
    fn a_matching_embedder_adds_the_semantic_tier_to_the_cascade() {
        let (_dir, verified, payload_key) = sealed_pack_with_semantic_index(
            thresholds_with_semantic(),
            SensitivityLevel::Confidential,
            Classification::Confidential,
        );
        let key = Arc::new(IndexKey::derive(&[31u8; 32], "semantic-load").expect("derive"));
        let embedder: Arc<dyn Embedder> = Arc::new(WordHashEmbedder {
            digest: EMBEDDER_DIGEST,
        });

        let loaded = load_pack(
            &verified,
            Some(&key),
            &PreResolvedKey::new(payload_key),
            Some(embedder),
        )
        .expect("a matching embedder must load");

        assert_eq!(
            loaded.cascade.tier_names().last(),
            Some(&"semantic"),
            "{:?}",
            loaded.cascade.tier_names()
        );
    }

    /// A semantic index with a matching embedder still refuses to load if the
    /// manifest never calibrated the semantic tier — a bare sentinel table is
    /// not a semantic one, and there is no default margin that would not be a
    /// fabricated threshold.
    #[test]
    fn a_semantic_index_with_no_semantic_thresholds_is_refused() {
        let bare_sentinel_table = serde_json::json!({
            "detector_version": "sentinel-0.1",
            "taxonomy_version": "1.0.0",
            "thresholds": {},
        });
        let (_dir, verified, payload_key) = sealed_pack_with_semantic_index(
            bare_sentinel_table,
            SensitivityLevel::Confidential,
            Classification::Confidential,
        );
        let key = Arc::new(IndexKey::derive(&[31u8; 32], "semantic-load").expect("derive"));
        let embedder: Arc<dyn Embedder> = Arc::new(WordHashEmbedder {
            digest: EMBEDDER_DIGEST,
        });

        let refused = load_pack(
            &verified,
            Some(&key),
            &PreResolvedKey::new(payload_key),
            Some(embedder),
        );

        assert!(matches!(refused, Err(LoadError::NoSemanticThresholds)));
    }

    /// KP-006: a recorded ceiling below the content it covers is a lie the
    /// policy pre-check would faithfully enforce, so the content gets the
    /// last word — mirroring `pack_test.rs`'s
    /// `a_ceiling_below_the_content_it_covers_is_refused`, but for an index
    /// whose *only* section is semantic. Without the
    /// `.max(indexes.semantic...)` fold in `open_indexes`'s content
    /// computation, this pack's content would read back `Public` (the empty
    /// reference index) and the ceiling check would pass it wrongly.
    #[test]
    fn a_semantic_ceiling_below_its_content_is_refused() {
        let (_dir, verified, payload_key) = sealed_pack_with_semantic_index(
            thresholds_with_semantic(),
            SensitivityLevel::Restricted,
            Classification::Internal,
        );
        let key = Arc::new(IndexKey::derive(&[31u8; 32], "semantic-load").expect("derive"));

        let refused = load_pack(
            &verified,
            Some(&key),
            &PreResolvedKey::new(payload_key),
            None,
        );

        let message = match refused {
            Err(e) => e.to_string(),
            Ok(_) => panic!("a semantic section recorded below its own content must not be loaded"),
        };
        assert!(message.contains("classified"), "{message}");
    }
}
