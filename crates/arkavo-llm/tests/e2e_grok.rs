//! End-to-end test against the **live Grok 4.6** xAI Responses API.
//!
//! Exercises the **router path** for `ModelChoice::Grok46`:
//! [`ResponsesProvider`] behind [`LlmClient`] with agent defaults
//! (`reasoning_effort: low`, `store: false` unless `XAI_STORE` is set).
//!
//! For Responses-native create (response id) and streaming, see
//! `e2e_xai_responses.rs`.
//!
//! ## Running it
//!
//! ```sh
//! export XAI_API_KEY=<your xAI api key>
//! cargo test -p arkavo-llm --test e2e_grok -- --ignored --nocapture
//! ```

#![allow(clippy::disallowed_methods)]

use arkavo_llm::providers::xai_responses::{ResponsesConfig, ResponsesProvider};
use arkavo_llm::{LlmClient, Message, Provider};

fn grok_provider() -> Option<ResponsesProvider> {
    let api_key = std::env::var("XAI_API_KEY").ok()?;
    let base_url =
        std::env::var("XAI_BASE_URL").unwrap_or_else(|_| "https://api.x.ai/v1".to_string());
    Some(
        ResponsesProvider::new(ResponsesConfig::for_agent(
            api_key,
            base_url,
            "grok-4.6".to_string(),
        ))
        .expect("ResponsesProvider construction should not fail with a valid config"),
    )
}

fn grok_client() -> Option<LlmClient> {
    Some(LlmClient::new(Box::new(grok_provider()?)))
}

fn is_transient_provider_error(err: &impl std::fmt::Display) -> bool {
    let s = err.to_string().to_lowercase();
    s.contains("429")
        || s.contains("too many requests")
        || s.contains("insufficient balance")
        || s.contains("rate limit")
        || s.contains("rate-limit")
        || s.contains("quota")
        || s.contains("timed out")
        || s.contains("timeout")
}

#[tokio::test]
#[ignore = "Requires XAI_API_KEY — makes a live Grok 4.6 Responses call"]
async fn grok46_round_trips_a_prompt() {
    let Some(client) = grok_client() else {
        eprintln!("XAI_API_KEY not set — skipping live Grok 4.6 e2e");
        return;
    };

    let result = client
        .complete(vec![Message::user(
            "Reply with exactly the word pong and nothing else.",
        )])
        .await;

    match result {
        Ok(text) => {
            assert!(
                text.to_lowercase().contains("pong"),
                "expected pong in response, got: {text}"
            );
        }
        Err(err) if is_transient_provider_error(&err) => {
            eprintln!("Skipping live Grok e2e on transient provider error: {err}");
        }
        Err(err) => panic!("Grok 4.6 round-trip failed: {err}"),
    }
}

#[tokio::test]
#[ignore = "Requires XAI_API_KEY — live Grok provider construction check"]
async fn grok46_provider_advertises_tools() {
    let Some(provider) = grok_provider() else {
        eprintln!("XAI_API_KEY not set — skipping Grok provider check");
        return;
    };
    assert!(
        provider.supports_tools(),
        "Grok Responses path must advertise tool support"
    );
    assert_eq!(provider.name(), "xai-responses");
}
