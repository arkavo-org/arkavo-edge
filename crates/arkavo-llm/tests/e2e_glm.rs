//! End-to-end test against the **live GLM-5.2** (Zhipu AI / Z.ai) endpoint.
//!
//! Exercises the exact path the router uses for `ModelChoice::Glm52`: the
//! generic OpenAI-compatible [`OpenAIProvider`] pointed at Z.ai's `paas/v4`
//! API, wrapped in [`LlmClient`] (identical to the CLI's GLM construction in
//! `arkavo-cli/src/commands/ui.rs`). GLM-5.2 speaks the OpenAI chat-completions
//! wire format, so no bespoke client is needed.
//!
//! This is the one thing a real key proves that CI can't: that a GLM-5.2 call
//! actually round-trips through our provider. It is `#[ignore]`-d and skips
//! cleanly when `GLM_API_KEY` is unset, so it never runs (or fails) in CI.
//!
//! ## Running it
//!
//! Put the token in the **environment** — never in code, args, or commits:
//!
//! ```sh
//! export GLM_API_KEY=<your z.ai api key>
//! # optional: mainland host
//! # export GLM_BASE_URL=https://open.bigmodel.cn/api/paas/v4
//! cargo test -p arkavo-llm --test e2e_glm -- --ignored --nocapture
//! ```

use arkavo_llm::providers::openai::{OpenAIConfig, OpenAIProvider};
use arkavo_llm::{LlmClient, Message};

/// Build the GLM-5.2 client from the environment, or `None` if the key is
/// absent (so the test skips rather than fails on a machine without a token).
fn glm_client() -> Option<LlmClient> {
    let api_key = std::env::var("GLM_API_KEY").ok()?;
    let base_url = std::env::var("GLM_BASE_URL")
        .unwrap_or_else(|_| "https://api.z.ai/api/paas/v4".to_string());
    let provider = OpenAIProvider::new(OpenAIConfig {
        api_key,
        base_url,
        model: "glm-5.2".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    })
    .expect("OpenAIProvider construction should not fail with a valid config");
    Some(LlmClient::new(Box::new(provider)))
}

#[tokio::test]
#[ignore = "Requires GLM_API_KEY — makes a live GLM-5.2 call to Z.ai"]
async fn glm52_round_trips_a_prompt() {
    let Some(client) = glm_client() else {
        eprintln!("GLM_API_KEY not set — skipping live GLM-5.2 e2e");
        return;
    };

    let messages = vec![Message::user(
        "What is 2 + 2? Respond with only the digit and nothing else.",
    )];

    let response = client
        .complete(messages)
        .await
        .expect("live GLM-5.2 completion should succeed");

    eprintln!("--- GLM-5.2 raw response ---\n{response}\n---------------------------");
    assert!(
        !response.trim().is_empty(),
        "GLM-5.2 returned empty content — the endpoint responded but produced no text"
    );
    assert!(
        response.contains('4'),
        "GLM-5.2 should answer 2+2 with a '4'; got: {response:?}"
    );
}
