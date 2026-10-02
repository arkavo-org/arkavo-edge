//! The per-model context bound, exercised through a provider and a real
//! model. Runs only when `ARKAVO_TEST_TEXT_MODEL` points at a GGUF, so an
//! ordinary `cargo test` stays offline:
//!
//!   ARKAVO_TEST_TEXT_MODEL=/path/to/model.gguf \
//!     cargo test -p arkavo-llm --features llama-cpp --test context_pool_bound_test
//!
//! One test function, because it counts the contexts alive in the process
//! and a second test creating contexts in parallel would change the count.

#![cfg(feature = "llama-cpp")]

use arkavo_llama_cpp::live_context_count;
use arkavo_llm::llamacpp_provider::{LlamaCppProvider, SamplingConfig};
use arkavo_llm::provider::Provider;
use arkavo_llm::{Error, Message, ModelRegistry, Role};
use std::sync::Arc;
use std::time::Duration;

const MODEL: &str = "bound-test";

fn provider(registry: &Arc<ModelRegistry>, wait: Duration) -> LlamaCppProvider {
    let config = SamplingConfig {
        temperature: 0.0,
        max_tokens: 8,
        ..Default::default()
    };
    LlamaCppProvider::new_with_registry(Arc::clone(registry), MODEL.to_string(), config)
        .expect("provider")
        .with_context_wait(wait)
}

fn question() -> Vec<Message> {
    vec![Message {
        role: Role::User,
        content: "Name one colour.".to_string(),
        ..Default::default()
    }]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_busy_model_queues_requests_on_its_one_context() {
    let Ok(path) = std::env::var("ARKAVO_TEST_TEXT_MODEL") else {
        eprintln!("skipping: set ARKAVO_TEST_TEXT_MODEL");
        return;
    };
    let before = live_context_count();
    let registry = Arc::new(ModelRegistry::with_max_contexts(1));
    registry.load(MODEL, &path).expect("load model");

    let held = registry
        .acquire_fresh_context(MODEL)
        .expect("the only context");
    assert_eq!(live_context_count(), before + 1);

    // A request made while the context is held waits for it.
    let waiting = {
        let provider = provider(&registry, Duration::from_secs(120));
        tokio::spawn(async move { provider.complete(question()).await })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !waiting.is_finished(),
        "request finished while the context was held"
    );
    assert_eq!(
        live_context_count(),
        before + 1,
        "a context was built outside the pool"
    );

    // A request that cannot wait long enough fails with a timeout, and
    // still builds nothing.
    let error = provider(&registry, Duration::from_millis(200))
        .complete(question())
        .await
        .expect_err("timed-out request");
    assert!(matches!(error, Error::Inference(_)), "got {error:?}");
    assert!(error.to_string().contains("Timed out"), "got {error}");
    assert_eq!(live_context_count(), before + 1);

    // Releasing the context lets the queued request run on it.
    registry
        .release_context(MODEL, held, true)
        .expect("release");
    waiting
        .await
        .expect("request task")
        .expect("queued request completes");
    assert_eq!(live_context_count(), before + 1);

    // The lease went back to the pool when generation finished.
    let stats = registry.context_pool().stats(MODEL).expect("pool stats");
    assert_eq!((stats.available, stats.in_use, stats.max), (1, 0, 1));
}
