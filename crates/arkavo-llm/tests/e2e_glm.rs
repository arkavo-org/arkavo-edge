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

use arkavo_budget::BudgetConfig;
use arkavo_budget::cost::TokenUsage;
use arkavo_budget::provider_costs::{PricingEntry, ProviderPricing};
use arkavo_budget::tracker::BudgetTracker;
use arkavo_llm::providers::openai::{OpenAIConfig, OpenAIProvider};
use arkavo_llm::{LlmClient, Message, Provider};

/// Build the GLM-5.2 client from the environment, or `None` if the key is
/// absent (so the test skips rather than fails on a machine without a token).
fn glm_provider() -> Option<OpenAIProvider> {
    let api_key = std::env::var("GLM_API_KEY").ok()?;
    let base_url = std::env::var("GLM_BASE_URL")
        .unwrap_or_else(|_| "https://api.z.ai/api/paas/v4".to_string());
    Some(
        OpenAIProvider::new(OpenAIConfig {
            api_key,
            base_url,
            model: "glm-5.2".to_string(),
            organization_id: None,
            api_version: None,
            is_azure: false,
        })
        .expect("OpenAIProvider construction should not fail with a valid config"),
    )
}

fn glm_client() -> Option<LlmClient> {
    Some(LlmClient::new(Box::new(glm_provider()?)))
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

/// Real GLM-5.2 call -> real `usage` tokens -> cost at the published per-MTok
/// rate -> through the budget gate. Validates the cost model against *actual*
/// spend (not a fixed category estimate), end to end.
#[tokio::test]
#[ignore = "Requires GLM_API_KEY — live GLM-5.2 call + cost reconciliation"]
async fn glm52_usage_reconciles_to_cost_and_budget() {
    let Some(provider) = glm_provider() else {
        eprintln!("GLM_API_KEY not set — skipping GLM-5.2 cost e2e");
        return;
    };

    let messages = vec![Message::user("Write a short haiku about budgets.")];
    let response = provider
        .complete_with_tools(messages, None, Some(128))
        .await
        .expect("live GLM-5.2 completion should succeed");

    // The new capability: real prompt/completion token counts surfaced from
    // Z.ai's `usage` block (previously dropped).
    let timing = response
        .inference_timing
        .expect("GLM-5.2 should surface token usage");
    assert!(
        timing.n_prompt_eval > 0 && timing.n_eval > 0,
        "expected real prompt+completion token counts, got {timing:?}"
    );

    // Price the real usage with the per-MTok table (published GLM-5.2 rate).
    let mut pricing = ProviderPricing::new();
    pricing.register(&PricingEntry {
        model_id: "glm-5.2".to_string(),
        provider: "zhipu".to_string(),
        input_cents_per_mtok: 140,
        output_cents_per_mtok: 440,
        cached_input_cents_per_mtok: Some(26),
        cache_write_cents_per_mtok: None,
        context_window: Some(1_000_000),
        max_output_tokens: Some(131_072),
    });
    let cost = pricing
        .estimate_cost("zhipu", "glm-5.2", timing.n_prompt_eval, timing.n_eval)
        .expect("GLM-5.2 must be priced from the per-MTok table");

    // Run the real spend through the budget gate and confirm it's tracked.
    let tracker = BudgetTracker::new(BudgetConfig::default()).await.unwrap();
    assert!(
        tracker.can_afford("glm-agent", cost).await.unwrap(),
        "a single GLM call must fit the default budget"
    );
    tracker
        .record_spending(
            "glm-agent".to_string(),
            "zhipu".to_string(),
            "glm-5.2".to_string(),
            TokenUsage::new(timing.n_prompt_eval, timing.n_eval),
            cost,
        )
        .await
        .unwrap();
    assert_eq!(tracker.get_status().await.session_spent, cost);

    eprintln!(
        "GLM-5.2 real spend: {} in + {} out tokens = {cost} ({} total)",
        timing.n_prompt_eval,
        timing.n_eval,
        timing.n_prompt_eval + timing.n_eval
    );
}
