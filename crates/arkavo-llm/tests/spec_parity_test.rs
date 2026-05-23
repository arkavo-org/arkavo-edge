//! Parity test: generation with NGRAM spec decoding must produce identical
//! tokens to generation without spec, at a fixed seed and temperature=0.0.
//! Any divergence is a correctness bug in the spec integration.
//!
//! Gated on `ARKAVO_TEST_MODEL` because it needs a real GGUF model. CI has
//! no model so this is `#[ignore]`d by default; run locally with:
//!
//!   ARKAVO_TEST_MODEL=$HOME/.arkavo/models/qwen3.5-9b-q4_k_m.gguf \
//!     cargo test -p arkavo-llm --test spec_parity_test -- --ignored --nocapture

#![cfg(all(feature = "llama-cpp", not(target_env = "musl")))]

use arkavo_llm::llamacpp_provider::{LlamaCppProvider, SamplingConfig};
use arkavo_llm::provider::Provider;
use arkavo_llm::{Message, Role};

fn make_provider(model_path: &str, use_spec: bool) -> LlamaCppProvider {
    let config = SamplingConfig {
        temperature: 0.0,
        seed: 42,
        max_tokens: 80,
        debug: false,
        use_spec_decoding: use_spec,
        ..Default::default()
    };
    LlamaCppProvider::new_with_config(
        "spec-parity-test".to_string(),
        model_path.to_string(),
        None,
        config,
    )
    .expect("Failed to load model for parity test")
}

fn make_messages() -> Vec<Message> {
    vec![Message {
        role: Role::User,
        content: "Write a JSON object with three keys: name, version, type. Output only the JSON."
            .to_string(),
        images: None,
        tool_call_id: None,
        tool_name: None,
    }]
}

#[tokio::test]
#[ignore = "requires local GGUF model; opt-in via ARKAVO_TEST_MODEL and --ignored"]
async fn ngram_spec_matches_baseline_output() {
    let model_path = match std::env::var("ARKAVO_TEST_MODEL") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("Skipping: set ARKAVO_TEST_MODEL=/path/to/model.gguf to run");
            return;
        }
    };

    let baseline_provider = make_provider(&model_path, false);
    let spec_provider = make_provider(&model_path, true);

    let baseline = baseline_provider
        .complete(make_messages())
        .await
        .expect("baseline completion failed");

    let spec = spec_provider
        .complete(make_messages())
        .await
        .expect("spec completion failed");

    assert_eq!(
        baseline, spec,
        "spec decoding diverged from baseline output:\n  baseline: {baseline:?}\n  spec: {spec:?}",
    );
}

/// Sanity check that the spec path actually drafted tokens on a
/// structured-output prompt. If `n_draft == 0` the parity test above
/// silently passes even with a broken integration, so this guards
/// against that footgun.
#[tokio::test]
#[ignore = "requires local GGUF model; opt-in via ARKAVO_TEST_MODEL and --ignored"]
async fn ngram_spec_produces_drafts_on_structured_prompt() {
    let model_path = match std::env::var("ARKAVO_TEST_MODEL") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("Skipping: set ARKAVO_TEST_MODEL=/path/to/model.gguf to run");
            return;
        }
    };

    let spec_provider = make_provider(&model_path, true);
    let response = spec_provider
        .complete_with_tools(make_messages(), None, None)
        .await
        .expect("spec completion failed");

    let timing = response
        .inference_timing
        .expect("spec path must populate inference_timing");
    let n_draft = timing.n_draft.unwrap_or(0);

    assert!(
        n_draft > 0,
        "spec should have drafted on structured output prompt, got n_draft={n_draft}; \
         n_accepted={:?}",
        timing.n_accepted,
    );
}
