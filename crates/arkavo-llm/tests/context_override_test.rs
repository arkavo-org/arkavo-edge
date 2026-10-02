//! `ARKAVO_N_CTX` end to end: the window the loader allocates is the window
//! generation stops at. Runs only when both a model and an override are
//! given, so an ordinary `cargo test` stays offline and never changes its
//! own environment:
//!
//!   ARKAVO_N_CTX=1024 ARKAVO_TEST_TEXT_MODEL=/path/to/model.gguf \
//!     cargo test -p arkavo-llm --features llama-cpp --test context_override_test

#![cfg(feature = "llama-cpp")]

use arkavo_llm::llamacpp_provider::{LlamaCppProvider, SamplingConfig};
use arkavo_llm::provider::Provider;
use arkavo_llm::{Error, Message, Role, ThinkingMode};

/// The model path and the override, or `None` when the test should skip.
fn setup() -> Option<(String, u32)> {
    let path = std::env::var("ARKAVO_TEST_TEXT_MODEL").ok()?;
    let n_ctx = std::env::var("ARKAVO_N_CTX").ok()?.parse().ok()?;
    Some((path, n_ctx))
}

fn provider(path: &str, max_tokens: u32) -> LlamaCppProvider {
    let config = SamplingConfig {
        temperature: 0.0,
        max_tokens,
        thinking_mode: Some(ThinkingMode::Off),
        ..Default::default()
    };
    LlamaCppProvider::new_with_config("override-test".to_string(), path.to_string(), None, config)
        .expect("load model")
}

fn user(content: String) -> Vec<Message> {
    vec![Message {
        role: Role::User,
        content,
        ..Default::default()
    }]
}

/// Regression: generation was clamped to a window recomputed from the
/// trained context (16,384 for most models), so with a smaller override it
/// ran off the end of the allocated KV cache and the request failed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generation_stops_at_the_overridden_window() {
    let Some((path, n_ctx)) = setup() else {
        eprintln!("skipping: set ARKAVO_TEST_TEXT_MODEL and ARKAVO_N_CTX");
        return;
    };
    // Asks for far more output than the window holds.
    let request = user(
        "Write the numbers from 1 to 3000 in words, one per line. Do not stop early.".to_string(),
    );

    let text = provider(&path, n_ctx * 4)
        .complete(request)
        .await
        .expect("generation ends at the window instead of failing");

    assert!(!text.trim().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_prompt_larger_than_the_window_is_refused_with_a_reason() {
    let Some((path, n_ctx)) = setup() else {
        eprintln!("skipping: set ARKAVO_TEST_TEXT_MODEL and ARKAVO_N_CTX");
        return;
    };
    // At least one token per repetition, so this cannot fit.
    let oversized = "alpha beta gamma delta ".repeat(n_ctx as usize);

    let error = provider(&path, 64)
        .complete(user(oversized))
        .await
        .expect_err("an oversized prompt cannot be decoded");

    assert!(matches!(error, Error::Inference(_)), "got {error:?}");
    let reason = error.to_string();
    assert!(reason.contains("too long"), "{reason}");
    assert!(reason.contains("ARKAVO_N_CTX"), "{reason}");
}
