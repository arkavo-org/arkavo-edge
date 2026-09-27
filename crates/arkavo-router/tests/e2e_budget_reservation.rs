//! End-to-end test for the live budget reservation gate.
//!
//! Exercises the full `route_with_budget` path — classify → select → reserve —
//! against a real `BudgetTracker`, proving the cost gate (a) reserves the
//! estimated cost atomically on a successful route and (b) **fails closed**
//! when an agent is over budget (the route is denied, not authorized).
//!
//! Local models are free, so the test authors a positive price for every
//! local arm; otherwise every reservation would be $0 and the gate could
//! never engage on cost. The classifier needs a local GGUF, so the test is
//! `#[ignore]` and soft-skips when no model is available.
//!
//! Run with:
//!   cargo test -p arkavo-router --features llama-cpp \
//!       --test e2e_budget_reservation -- --ignored --nocapture

#![cfg(feature = "llama-cpp")]
// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lints as
// a disallowed method; allowed here as in the other `e2e_*` integration tests.
#![allow(clippy::disallowed_methods)]

use arkavo_budget::config::AgentBudget;
use arkavo_budget::provider_costs::{PricingEntry, ProviderPricing};
use arkavo_budget::{BudgetConfig, BudgetManager, TokenCost};
use arkavo_router::orchestrator::CostOrchestrator;
use arkavo_test_macros::spec;
use std::collections::HashMap;

/// Every local arm priced at $1.40/$4.40 per MTok (the published GLM-5.2 rate),
/// so a category-sized reservation costs a few cents — well above the 1¢
/// per-agent cap used below and well under the $10 global limit.
fn local_pricing() -> ProviderPricing {
    const LOCAL_MODELS: &[(&str, &str)] = &[
        ("local-qwen", "qwen3.5-0.8b"),
        ("local-qwen", "qwen3.5-9b"),
        ("local-qwen", "qwen3.5-27b"),
        ("local-qwen", "qwen3.6-35b-a3b"),
        ("local-ministral", "ministral-3b"),
        ("local-ministral", "ministral-8b"),
        ("local-glm", "glm-4.7-flash"),
        ("local-gemma", "gemma-4-e2b"),
        ("local-gemma", "gemma-4-e4b"),
        ("local-gemma", "gemma-4-26b-a4b"),
        ("local-gemma", "gemma-4-31b"),
        ("local-gemma", "gemma-4-12b"),
        ("local-gemma", "gemma-3-270m-it"),
        ("local-gemma", "gemma-3-4b-it"),
        ("local-gemma", "gemma-3-12b-it"),
        ("local-deepseek", "deepseek-coder-v2-lite-instruct"),
    ];
    let mut pricing = ProviderPricing::new();
    for (provider, model) in LOCAL_MODELS {
        pricing.register(&PricingEntry {
            model_id: (*model).to_string(),
            provider: (*provider).to_string(),
            input_cents_per_mtok: 1400,
            output_cents_per_mtok: 4400,
            cached_input_cents_per_mtok: None,
            cache_write_cents_per_mtok: None,
            context_window: None,
            max_output_tokens: None,
        });
    }
    pricing
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[spec("BUDGET-009")]
#[ignore = "requires a local GGUF model for the classifier"]
async fn budget_gate_reserves_and_fails_closed_over_budget() {
    // Global $10 session limit (BudgetConfig default); one agent capped at 1¢.
    let mut agent_budgets = HashMap::new();
    agent_budgets.insert(
        "broke-agent".to_string(),
        AgentBudget::new("broke-agent".to_string()).with_session_limit(TokenCost::from_cents(1)),
    );
    let config = BudgetConfig {
        agent_budgets,
        ..Default::default()
    };

    let manager = BudgetManager::new(config).await.unwrap();
    let tracker = manager.tracker();

    let orchestrator =
        match CostOrchestrator::new_with_pricing(tracker.clone(), local_pricing()).await {
            Ok(o) => o,
            Err(e) => {
                eprintln!("Skipping E2E budget test: classifier model unavailable ({e})");
                return;
            }
        };

    let task = "write a function that adds two numbers";

    // rich-agent has no per-agent cap, only the $10 global — the route is
    // allowed and its estimated cost is reserved (a spending record is left).
    let rich = orchestrator.route_with_budget(task, "rich-agent").await;
    assert!(
        rich.is_ok(),
        "rich-agent is well within the $10 global budget and must be allowed: {rich:?}"
    );
    let after_rich = tracker.get_spending_history(32).await;
    assert!(
        after_rich.iter().any(|r| r.agent_id == "rich-agent"),
        "a successful route must reserve the estimated cost (leave a spending record)"
    );
    let rich_reserved = after_rich
        .iter()
        .find(|r| r.agent_id == "rich-agent")
        .map(|r| r.cost)
        .unwrap();
    assert!(
        rich_reserved > TokenCost::ZERO,
        "with authored local pricing the reservation must be > $0, got {rich_reserved}"
    );

    // broke-agent is capped at 1¢; the cheapest priced reservation is several
    // cents, so the gate must DENY it — fail closed — and leave no record.
    let broke = orchestrator.route_with_budget(task, "broke-agent").await;
    assert!(
        broke.is_err(),
        "broke-agent is over its 1¢ cap; the gate must fail closed and deny, got {broke:?}"
    );
    let after_broke = tracker.get_spending_history(32).await;
    assert!(
        !after_broke.iter().any(|r| r.agent_id == "broke-agent"),
        "a denied route must not reserve any budget (no spending record for the denied agent)"
    );
}
