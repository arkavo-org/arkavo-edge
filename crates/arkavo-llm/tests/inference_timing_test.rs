//! The decode timing a local response reports, through a provider and a real
//! model. Runs only when `ARKAVO_TEST_TEXT_MODEL` points at a GGUF, so an
//! ordinary `cargo test` stays offline:
//!
//!   ARKAVO_TEST_TEXT_MODEL=/path/to/model.gguf \
//!     cargo test -p arkavo-llm --features llama-cpp --test inference_timing_test

#![cfg(feature = "llama-cpp")]
#![allow(clippy::disallowed_methods)] // tokio::test uses block_on internally

use arkavo_llm::llamacpp_provider::{LlamaCppProvider, SamplingConfig};
use arkavo_llm::provider::{InferenceTiming, Provider};
use arkavo_llm::{Message, ModelRegistry, Role};
use std::sync::Arc;

const MODEL: &str = "timing-test";
const MAX_TOKENS: u32 = 8;

fn question() -> Vec<Message> {
    vec![Message {
        role: Role::User,
        content: "Name one colour.".to_string(),
        ..Default::default()
    }]
}

async fn timing(provider: &LlamaCppProvider) -> InferenceTiming {
    provider
        .complete_with_tools(question(), None, None)
        .await
        .expect("completion")
        .inference_timing
        .expect("a local response reports its timing")
}

/// Regression: contexts were created with llama.cpp's default `no_perf`, so
/// every response reported 0 ms, and a pooled context's counters were never
/// reset, so each response reported every request the context had served.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_pooled_response_reports_its_own_decode_time() {
    let Ok(path) = std::env::var("ARKAVO_TEST_TEXT_MODEL") else {
        eprintln!("skipping: set ARKAVO_TEST_TEXT_MODEL");
        return;
    };
    let registry = Arc::new(ModelRegistry::with_max_contexts(1));
    registry.load(MODEL, &path).expect("load model");
    let config = SamplingConfig {
        temperature: 0.0,
        max_tokens: MAX_TOKENS,
        ..Default::default()
    };
    let provider =
        LlamaCppProvider::new_with_registry(Arc::clone(&registry), MODEL.to_string(), config)
            .expect("provider");

    let first = timing(&provider).await;
    let second = timing(&provider).await;

    for t in [&first, &second] {
        assert!(t.prompt_eval_ms > 0.0, "{t:?}");
        assert!(t.generation_ms > 0.0, "{t:?}");
        assert!(
            t.n_eval <= MAX_TOKENS,
            "one response's tokens, not the context's: {t:?}"
        );
    }
    // llama.cpp's own count of the same prompt can differ by a token between
    // runs; carried-over counters would at least double it.
    assert!(
        second.n_prompt_eval < 2 * first.n_prompt_eval,
        "the second response counted the first one's prompt too: {first:?} then {second:?}"
    );
}
