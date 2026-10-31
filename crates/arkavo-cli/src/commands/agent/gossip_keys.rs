//! Binding a discovered peer's gossip key (INGRESS-004, INGRESS-017).
//!
//! Each side proves it holds its gossip key and shows the statement in
//! which its workload key vouches for it. A peer from before key statements
//! shows none and is bound unvouched, as before.

use arkavo_gossip::key_exchange::{KeyExchangeResponse, sign_request, verify_response};
use arkavo_protocol::http::HttpTransport;
use arkavo_protocol::transport::{
    A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TransportConfig,
};
use arkavo_server::LearningBus;

/// Exchange gossip keys with `peer_id` at `address` and bind the key its
/// reply proves. Returns the workload `did:key` that vouched for it.
pub(super) async fn exchange(
    bus: &LearningBus,
    peer_id: &str,
    address: &str,
) -> Result<Option<String>, String> {
    // Prove we hold our key, addressed to this peer, and keep the challenge
    // its reply must sign.
    let (mut request, challenge) = sign_request(bus.keypair(), bus.agent_id(), peer_id);
    request.statement = bus.gossip_statement().await;
    let params = serde_json::to_value(&request).map_err(|e| e.to_string())?;

    let mut config = TransportConfig::default();
    config.tls_config.require_tls = false;
    let transport = HttpTransport::new(config).map_err(|e| e.to_string())?;
    transport
        .connect(&A2aEndpoint {
            url: address.to_string(),
            agent_id: peer_id.to_string(),
            public_key: None,
        })
        .await
        .map_err(|e| e.to_string())?;
    let response = transport
        .send_request(A2aRequest::new("agent/exchangeKeys", params))
        .await
        .map_err(|e| e.to_string())?;
    let A2aResponse::Success { result, .. } = response else {
        return Err(format!("{peer_id} refused the key exchange"));
    };

    // Bind their key only once their reply proves they hold it.
    let reply: KeyExchangeResponse = serde_json::from_value(result).map_err(|e| e.to_string())?;
    let their_key =
        verify_response(&reply, peer_id, bus.agent_id(), &challenge).map_err(|e| e.to_string())?;
    bus.bind_peer_key(peer_id.to_string(), their_key, reply.statement.as_deref())
        .await
        .map_err(|e| e.to_string())
}
