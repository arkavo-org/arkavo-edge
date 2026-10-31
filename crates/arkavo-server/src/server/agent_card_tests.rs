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

/// The card goes out signed by the agent's workload key, and the signature
/// names that key's DID (SRV-017).
#[arkavo_test_macros::spec("SRV-017")]
#[tokio::test]
async fn the_card_is_signed_by_the_agent_workload_key() {
    let dir = tempfile::tempdir().unwrap();
    let key =
        arkavo_device_identity::workload_key::load_or_create_at(&dir.path().join("workload_key"))
            .unwrap();
    let did = key.did().to_string();
    let mut rpc = rpc_impl(Arc::new(RwLock::new(planner(Some("Plans the work"))))).await;
    rpc.workload_key = Some(Arc::new(key));
    let card = rpc
        .into_rpc()
        .call::<[(); 0], serde_json::Value>("agent_card", [])
        .await
        .expect("the agent card is always served");

    assert_eq!(arkavo_protocol::card_signature::verify(&card), Ok(did));
}

#[tokio::test]
async fn an_agent_without_a_workload_key_serves_its_card_unsigned() {
    let card = served_card(planner(Some("Plans the work"))).await;
    assert!(card.get("signatures").is_none(), "{card}");
}

/// The signed card carries the statements in which the workload key vouches
/// for the agent's gossip, TDF and iroh keys (SRV-018).
#[arkavo_test_macros::spec("SRV-018")]
#[tokio::test]
async fn the_card_carries_a_statement_for_each_of_the_agents_keys() {
    use arkavo_device_identity::subordinate_key::{self, Purpose, Voucher};

    let dir = tempfile::tempdir().unwrap();
    let workload = Arc::new(
        arkavo_device_identity::workload_key::load_or_create_at(&dir.path().join("workload_key"))
            .unwrap(),
    );
    let mut keys = Vec::new();
    let mut vouchers = Vec::new();
    for purpose in [Purpose::Gossip, Purpose::Tdf, Purpose::Iroh] {
        let key =
            subordinate_key::load_or_create_at(&dir.path().join(format!("{purpose:?}")), purpose)
                .unwrap();
        vouchers.push(Arc::new(Voucher::new(workload.clone(), &key)));
        keys.push((purpose, key.did().to_string()));
    }
    let mut rpc = rpc_impl(Arc::new(RwLock::new(planner(Some("Plans the work"))))).await;
    rpc.workload_key = Some(workload.clone());
    rpc.key_vouchers = vouchers;
    let card = rpc
        .into_rpc()
        .call::<[(); 0], serde_json::Value>("agent_card", [])
        .await
        .expect("the agent card is always served");

    // The statements are inside what the card signature covers.
    assert_eq!(
        arkavo_protocol::card_signature::verify(&card),
        Ok(workload.did().to_string())
    );
    let extension = card["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["uri"] == arkavo_key_statement::CARD_EXTENSION)
        .expect("the card carries the key statements extension");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let vouched: Vec<(Purpose, String)> = extension["data"]["statements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|compact| {
            let statement = arkavo_key_statement::verify(compact.as_str().unwrap(), now).unwrap();
            assert_eq!(statement.issuer, workload.did());
            (statement.purpose, statement.key)
        })
        .collect();
    assert_eq!(vouched, keys);
}

#[tokio::test]
async fn an_agent_with_no_other_keys_carries_no_statements() {
    let dir = tempfile::tempdir().unwrap();
    let key =
        arkavo_device_identity::workload_key::load_or_create_at(&dir.path().join("workload_key"))
            .unwrap();
    let mut rpc = rpc_impl(Arc::new(RwLock::new(planner(Some("Plans the work"))))).await;
    rpc.workload_key = Some(Arc::new(key));
    let card = rpc
        .into_rpc()
        .call::<[(); 0], serde_json::Value>("agent_card", [])
        .await
        .expect("the agent card is always served");
    assert!(
        card.get("extensions")
            .is_none_or(|e| e.as_array().is_some_and(Vec::is_empty)),
        "{card}"
    );
}
