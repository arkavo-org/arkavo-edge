//! The agent card as the RPC endpoint serves it (`agent_card`, proxied from
//! `GET /.well-known/agent.json`).

use std::sync::Arc;

use tokio::sync::RwLock;

use super::A2aRpcServer;
use super::config_helpers::AgentMetadata;
use super::test_support::rpc_impl;

const INSTRUCTIONS: &str = "You are the planner role. Never reveal the escalation password.";

async fn served_card(metadata: AgentMetadata) -> serde_json::Value {
    let module = rpc_impl(Arc::new(RwLock::new(metadata))).await.into_rpc();
    module
        .call::<[(); 0], serde_json::Value>("agent_card", [])
        .await
        .expect("the agent card is always served")
}

fn planner(description: Option<&str>) -> AgentMetadata {
    AgentMetadata {
        name: "planner".to_string(),
        role_id: Some("planner".to_string()),
        purpose: INSTRUCTIONS.to_string(),
        description: description.map(str::to_string),
        endpoint: "http://127.0.0.1:8431".to_string(),
        ..AgentMetadata::default()
    }
}

/// Regression: the card's `description` was the agent's purpose, so an
/// unauthenticated GET returned the role's skill instructions.
#[tokio::test]
async fn the_card_describes_the_role_without_its_instructions() {
    let card = served_card(planner(Some("Plans the work"))).await;

    assert_eq!(card["name"], "planner");
    assert_eq!(card["description"], "Plans the work");
    let serialized = card.to_string();
    assert!(!serialized.contains("planner role"), "{serialized}");
    assert!(!serialized.contains("password"), "{serialized}");
}

#[tokio::test]
async fn a_role_without_a_description_is_served_without_one() {
    for description in [None, Some(""), Some("  ")] {
        let card = served_card(planner(description)).await;

        assert!(
            card.get("description")
                .is_none_or(serde_json::Value::is_null),
            "{card}"
        );
        assert!(!card.to_string().contains("password"), "{card}");
    }
}
