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
                response_items: Vec::new(),
                content: String::from_utf8_lossy(c).to_string(),
                reasoning_content: None,
                done: false,
                inference_timing: None,
            })
        })
        .collect();
    chunks.push(Ok(StreamResponse {
        response_items: Vec::new(),
        content: String::new(),
        reasoning_content: None,
        done: true,
        inference_timing: None,
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
    let runtime =
        SentinelRuntime::from_pack(&verified, Some(&key), &PreResolvedKey::new(payload_key))
            .expect("provision from the pack");

    // SENT-004: the thresholds came out of the signed manifest.
    assert_eq!(runtime.calibration.detector_version, "sentinel-0.1");

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
