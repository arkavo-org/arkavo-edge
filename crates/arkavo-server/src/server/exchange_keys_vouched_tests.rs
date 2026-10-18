//! `agent/exchangeKeys` carrying the key statements in which each side's
//! workload key vouches for its gossip key (INGRESS-017).

use std::path::Path;
use std::sync::Arc;

use arkavo_crypto::AgentKeypair;
use arkavo_device_identity::subordinate_key::{self, Purpose, SubordinateKey, Voucher};
use arkavo_device_identity::workload_key::{self, WorkloadKey};
use arkavo_gossip::GossipConfig;
use arkavo_gossip::key_exchange::{
    KeyExchangeRequest, KeyExchangeResponse, sign_request, verify_response, vouched_by,
};
use arkavo_test_macros::spec;
use jsonrpsee::RpcModule;
use jsonrpsee::core::params::ObjectParams;
use tokio::sync::RwLock;

use super::A2aRpcServer;
use super::config_helpers::AgentMetadata;
use super::learning_bus::LearningBus;
use super::test_support::rpc_impl;

const AGENT: &str = "beta";

/// An agent's kept workload key and gossip key, in `dir`.
struct Agent {
    workload: Arc<WorkloadKey>,
    gossip_key: SubordinateKey,
    keypair: AgentKeypair,
}

impl Agent {
    fn in_dir(dir: &Path) -> Self {
        let workload =
            Arc::new(workload_key::load_or_create_at(&dir.join("workload_key")).unwrap());
        let gossip_key =
            subordinate_key::load_or_create_at(&dir.join("gossip_key"), Purpose::Gossip).unwrap();
        let keypair = AgentKeypair::from_bytes(gossip_key.secret()).unwrap();
        Self {
            workload,
            gossip_key,
            keypair,
        }
    }

    fn voucher(&self) -> Arc<Voucher> {
        Arc::new(Voucher::new(self.workload.clone(), &self.gossip_key))
    }

    fn statement(&self) -> String {
        self.voucher().statement().unwrap()
    }

    fn request(&self, peer_id: &str) -> (KeyExchangeRequest, Vec<u8>) {
        let (mut request, challenge) = sign_request(&self.keypair, peer_id, AGENT);
        request.statement = Some(self.statement());
        (request, challenge)
    }
}

/// Named params, as the CLI sends them.
fn params(request: &KeyExchangeRequest) -> ObjectParams {
    let mut params = ObjectParams::new();
    let value = serde_json::to_value(request).unwrap();
    for (name, field) in value.as_object().expect("an object").iter() {
        params.insert(name, field).unwrap();
    }
    params
}

async fn serving(agent: &Agent) -> (RpcModule<super::A2aRpcImpl>, Arc<LearningBus>) {
    let bus = LearningBus::new(
        AGENT.to_string(),
        "test-swarm".to_string(),
        Arc::new(AgentKeypair::from_bytes(agent.gossip_key.secret()).unwrap()),
        GossipConfig::default(),
    );
    bus.set_gossip_voucher(agent.voucher());
    let bus = Arc::new(bus);
    let mut rpc = rpc_impl(Arc::new(RwLock::new(AgentMetadata::default()))).await;
    rpc.learning_bus = Some(bus.clone());
    (rpc.into_rpc(), bus)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[spec("INGRESS-017")]
#[tokio::test]
async fn each_side_learns_which_agent_the_other_is() {
    let dir = tempfile::tempdir().unwrap();
    let serving_agent = Agent::in_dir(&dir.path().join("beta"));
    let caller = Agent::in_dir(&dir.path().join("alpha"));
    let (module, bus) = serving(&serving_agent).await;

    let (request, challenge) = caller.request("alpha");
    let reply: KeyExchangeResponse = module
        .call("agent/exchangeKeys", params(&request))
        .await
        .expect("a vouched request is accepted");

    let their_key = verify_response(&reply, AGENT, "alpha", &challenge).unwrap();
    let vouched = vouched_by(reply.statement.as_deref(), &their_key, now())
        .unwrap()
        .expect("the reply carries the agent's statement");
    assert_eq!(vouched.issuer, serving_agent.workload.did());

    // The caller's identity was recorded: a goodbye does not free its name.
    bus.remove_peer("alpha").await;
    let err = bus
        .bind_peer_key("alpha".into(), AgentKeypair::generate().public_key(), None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains(caller.workload.did()), "{err}");
}

/// The agent restarts with the gossip key it kept, and its peer binds it
/// again instead of refusing a new key under its name.
#[spec("INGRESS-017")]
#[tokio::test]
async fn a_restarted_agent_binds_again() {
    let dir = tempfile::tempdir().unwrap();
    let (module, bus) = serving(&Agent::in_dir(&dir.path().join("beta"))).await;
    let first = Agent::in_dir(&dir.path().join("alpha"));
    module
        .call::<_, KeyExchangeResponse>("agent/exchangeKeys", params(&first.request("alpha").0))
        .await
        .unwrap();
    bus.remove_peer("alpha").await;

    let restarted = Agent::in_dir(&dir.path().join("alpha"));
    module
        .call::<_, KeyExchangeResponse>("agent/exchangeKeys", params(&restarted.request("alpha").0))
        .await
        .expect("the kept key binds again");
}

/// Agents from before key statements send none, and are bound as before.
#[spec("INGRESS-017")]
#[tokio::test]
async fn an_agent_from_before_key_statements_still_binds() {
    let dir = tempfile::tempdir().unwrap();
    let (module, _) = serving(&Agent::in_dir(&dir.path().join("beta"))).await;
    let old = AgentKeypair::generate();
    let (request, challenge) = sign_request(&old, "alpha", AGENT);
    assert!(request.statement.is_none());

    let reply: KeyExchangeResponse = module
        .call("agent/exchangeKeys", params(&request))
        .await
        .expect("an unvouched request is accepted");
    assert!(verify_response(&reply, AGENT, "alpha", &challenge).is_ok());
}

/// A statement that does not hold is refused, not dropped: otherwise a
/// damaged statement would bind as an agent from before key statements.
#[spec("INGRESS-017")]
#[tokio::test]
async fn a_statement_for_another_key_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (module, bus) = serving(&Agent::in_dir(&dir.path().join("beta"))).await;
    let caller = Agent::in_dir(&dir.path().join("alpha"));
    let stranger = AgentKeypair::generate();
    let (mut request, _) = sign_request(&stranger, "alpha", AGENT);
    request.statement = Some(caller.statement());

    let result = module
        .call::<_, KeyExchangeResponse>("agent/exchangeKeys", params(&request))
        .await;
    let error = result.expect_err("a statement for another key is refused");
    assert!(error.to_string().contains("another key"), "{error}");
    bus.register_peer_key("alpha".into(), stranger.public_key())
        .await
        .expect("nothing was bound");
}

/// A voucher for another key would have the agent vouch for a key it does
/// not sign gossip with; the bus shows no statement instead.
#[spec("INGRESS-017")]
#[tokio::test]
async fn a_voucher_for_another_key_is_not_shown() {
    let dir = tempfile::tempdir().unwrap();
    let agent = Agent::in_dir(&dir.path().join("beta"));
    let bus = LearningBus::new(
        AGENT.to_string(),
        "test-swarm".to_string(),
        Arc::new(AgentKeypair::generate()),
        GossipConfig::default(),
    );
    bus.set_gossip_voucher(agent.voucher());
    assert_eq!(bus.gossip_statement().await, None);
}
