//! Who an agent proves it is: the `did:key` that signed its card (SRV-017).
//!
//! The gateway reads the card once per connection and checks its signature
//! itself, so a view shows an identity only when the agent's key vouches for
//! it. The check proves which key speaks for the agent, not where that key is
//! kept: a Mac cannot attest that a key lives in its Secure Enclave, so views
//! do not claim it.

use super::AgentConnection;
use arkavo_protocol::card_signature::{self, CardSignatureError};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What an agent's card proves about the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CardIdentity {
    /// The card is signed by the key its `did:key` names.
    Verified { did: String },
    /// The agent serves an unsigned card, as agents before card signing do.
    Unsigned,
    /// The card carries a signature that does not check out.
    Invalid { reason: String },
}

impl CardIdentity {
    pub fn of(card: &Value) -> Self {
        match card_signature::verify(card) {
            Ok(did) => Self::Verified { did },
            Err(CardSignatureError::Unsigned) => Self::Unsigned,
            Err(e) => Self::Invalid {
                reason: e.to_string(),
            },
        }
    }
}

impl AgentConnection {
    /// What the agent's card proved when this connection last read it, or
    /// nothing while it has not been read.
    pub async fn identity(&self) -> Option<CardIdentity> {
        self.identity.read().await.clone()
    }

    /// Read the agent's card and check its signature. An agent that does not
    /// answer leaves its identity unreported rather than failed.
    pub(super) async fn read_identity(&self) {
        let card = match &*self.client.read().await {
            Some(client) => client
                .request::<Value, _>("agent_card", rpc_params![])
                .await
                .ok(),
            None => None,
        };
        *self.identity.write().await = card.as_ref().map(CardIdentity::of);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_crypto::P256SigningKeypair;
    use arkavo_protocol::types::{AgentCapabilities, AgentCard};
    use arkavo_test_macros::spec;
    use jsonrpsee::RpcModule;
    use jsonrpsee::server::{Server, ServerHandle};
    use tokio::sync::mpsc;

    fn card() -> AgentCard {
        AgentCard {
            name: "planner".to_string(),
            description: None,
            url: "http://127.0.0.1:1".to_string(),
            provider: None,
            version: "0.107.0".to_string(),
            protocol_versions: vec!["0.3".to_string()],
            default_input_modes: vec!["text/plain".to_string()],
            default_output_modes: vec!["text/plain".to_string()],
            capabilities: AgentCapabilities::default(),
            skills: vec![],
            security_schemes: vec![],
            security: vec![],
            extensions: vec![],
            signatures: vec![],
        }
    }

    fn signed_by(keypair: &P256SigningKeypair) -> Value {
        let did = keypair.public_key().to_did_key();
        let kid = format!("{did}#{}", did.trim_start_matches("did:key:"));
        let mut card = card();
        card_signature::sign(&mut card, &kid, |input| {
            Ok::<_, String>(keypair.sign_p1363(input))
        })
        .unwrap();
        serde_json::to_value(card).unwrap()
    }

    async fn agent_serving(card: Value) -> (ServerHandle, String) {
        let mut module = RpcModule::new(());
        module
            .register_method("agent_card", move |_, (), _| card.clone())
            .unwrap();
        let server = Server::builder().build("127.0.0.1:0").await.unwrap();
        let endpoint = server.local_addr().unwrap().to_string();
        (server.start(module), endpoint)
    }

    async fn identity_of(card: Value) -> Option<CardIdentity> {
        let (_server, endpoint) = agent_serving(card).await;
        let (telemetry_tx, _telemetry_rx) = mpsc::channel(4);
        let connection = AgentConnection::new("planner".to_string(), endpoint, telemetry_tx);
        // Connecting reads the card; nothing else is needed.
        connection.connect().await.expect("connects");
        connection.identity().await
    }

    #[spec("SRV-017")]
    #[tokio::test]
    async fn a_signed_card_shows_the_did_that_signed_it() {
        let keypair = P256SigningKeypair::generate();
        assert_eq!(
            identity_of(signed_by(&keypair)).await,
            Some(CardIdentity::Verified {
                did: keypair.public_key().to_did_key()
            })
        );
    }

    #[spec("SRV-017")]
    #[tokio::test]
    async fn a_card_changed_after_signing_is_shown_as_invalid() {
        let mut card = signed_by(&P256SigningKeypair::generate());
        card["name"] = Value::String("someone else".to_string());
        assert!(matches!(
            identity_of(card).await,
            Some(CardIdentity::Invalid { .. })
        ));
    }

    #[spec("SRV-017")]
    #[tokio::test]
    async fn an_agent_from_before_card_signing_is_shown_as_unsigned() {
        let card = serde_json::to_value(card()).unwrap();
        assert_eq!(identity_of(card).await, Some(CardIdentity::Unsigned));
    }

    #[tokio::test]
    async fn an_agent_that_does_not_answer_leaves_its_identity_unreported() {
        let (telemetry_tx, _telemetry_rx) = mpsc::channel(4);
        let connection = AgentConnection::new(
            "planner".to_string(),
            "127.0.0.1:1".to_string(),
            telemetry_tx,
        );
        connection.read_identity().await;
        assert_eq!(connection.identity().await, None);
    }
}
