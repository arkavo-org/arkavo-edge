//! `agent/exchangeKeys` as the RPC endpoint serves it.

use std::sync::Arc;

use arkavo_crypto::AgentKeypair;
use arkavo_gossip::GossipConfig;
use arkavo_gossip::key_exchange::{KeyExchangeResponse, sign_request, verify_response};
use arkavo_test_macros::spec;
use jsonrpsee::RpcModule;
use jsonrpsee::core::params::ObjectParams;
use tokio::sync::RwLock;

use super::A2aRpcServer;
use super::config_helpers::AgentMetadata;
use super::learning_bus::LearningBus;
use super::test_support::rpc_impl;

const AGENT: &str = "beta";

/// Named params, as the CLI sends them.
fn params(value: serde_json::Value) -> ObjectParams {
    let mut params = ObjectParams::new();
    for (name, field) in value.as_object().expect("an object").iter() {
        params.insert(name, field).unwrap();
    }
    params
}

async fn module() -> (RpcModule<super::A2aRpcImpl>, Arc<LearningBus>) {
    let bus = Arc::new(LearningBus::new(
        AGENT.to_string(),
        "test-swarm".to_string(),
        Arc::new(AgentKeypair::generate()),
        GossipConfig::default(),
    ));
    let mut rpc = rpc_impl(Arc::new(RwLock::new(AgentMetadata::default()))).await;
    rpc.learning_bus = Some(bus.clone());
    (rpc.into_rpc(), bus)
}

#[spec("INGRESS-004")]
#[tokio::test]
async fn a_proven_exchange_binds_both_keys() {
    let (module, bus) = module().await;
    let caller = AgentKeypair::generate();
    let (request, challenge) = sign_request(&caller, "alpha", AGENT);

    let reply: KeyExchangeResponse = module
        .call(
            "agent/exchangeKeys",
            params(serde_json::to_value(&request).unwrap()),
        )
        .await
        .expect("a proven request is accepted");

    let agent_key = verify_response(&reply, AGENT, "alpha", &challenge).unwrap();
    assert_eq!(agent_key.to_bytes(), bus.keypair().public_key().to_bytes());
    // Binding the same key again is accepted; it is how a retry looks.
    bus.register_peer_key("alpha".into(), caller.public_key())
        .await
        .unwrap();
}

/// Regression: the old two-field request bound any key to any id.
#[spec("INGRESS-004")]
#[tokio::test]
async fn an_unproven_request_is_refused() {
    let (module, bus) = module().await;
    let claimed = AgentKeypair::generate().public_key().to_base64();
    let old_style = serde_json::json!({ "peer_id": "alpha", "public_key": claimed });

    let result = module
        .call::<_, serde_json::Value>("agent/exchangeKeys", params(old_style))
        .await;
    assert!(result.is_err());
    // Nothing was bound, so a proven key for the id is still accepted.
    bus.register_peer_key("alpha".into(), AgentKeypair::generate().public_key())
        .await
        .unwrap();
}

/// Regression: a second caller replaced a peer's bound key and could then
/// sign gossip as that peer.
#[spec("INGRESS-004")]
#[tokio::test]
async fn a_bound_peer_key_is_not_replaced() {
    let (module, _) = module().await;
    let peer = AgentKeypair::generate();
    let (first, _) = sign_request(&peer, "alpha", AGENT);
    module
        .call::<_, KeyExchangeResponse>(
            "agent/exchangeKeys",
            params(serde_json::to_value(&first).unwrap()),
        )
        .await
        .unwrap();

    let impostor = AgentKeypair::generate();
    let (second, _) = sign_request(&impostor, "alpha", AGENT);
    let result = module
        .call::<_, KeyExchangeResponse>(
            "agent/exchangeKeys",
            params(serde_json::to_value(&second).unwrap()),
        )
        .await;
    let error = result.expect_err("a different key for a bound peer is refused");
    assert!(error.to_string().contains("already bound"), "{error}");
}
