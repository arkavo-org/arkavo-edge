//! Who an agent proves it is: the `did:key` that signed its card (SRV-017).
//!
//! The gateway reads the card once per connection and checks its signature
//! itself, so a view shows an identity only when the agent's key vouches for
//! it. The check proves which key speaks for the agent, not where that key is
//! kept: a Mac cannot attest that a key lives in its Secure Enclave, so views
//! do not claim it. The gateway also checks the key statements the card
//! carries (SRV-018) and lists the keys the agent's identity vouches for.

use super::AgentConnection;
use arkavo_key_statement::Purpose;
use arkavo_protocol::card_signature::{self, CardSignatureError};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What an agent's card proves about the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CardIdentity {
    /// The card is signed by the key its `did:key` names, which vouches for
    /// `keys`.
    Verified {
        did: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        keys: Vec<VouchedKey>,
    },
    /// The agent serves an unsigned card, as agents before card signing do.
    Unsigned,
    /// The card carries a signature that does not check out.
    Invalid { reason: String },
}

/// A key the agent uses for one job, vouched for by its workload key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VouchedKey {
    pub purpose: Purpose,
    /// The key's `did:key`.
    pub key: String,
    /// Unix seconds when the statement stops holding; the agent renews it
    /// before then.
    pub valid_until: u64,
}

impl CardIdentity {
    pub fn of(card: &Value) -> Self {
        match card_signature::verify(card) {
            Ok(did) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_secs());
                let keys = vouched_keys(card, &did, now);
                Self::Verified { did, keys }
            }
            Err(CardSignatureError::Unsigned) => Self::Unsigned,
            Err(e) => Self::Invalid {
                reason: e.to_string(),
            },
        }
    }
}

/// The keys `did` vouches for in the card's key statements. A statement
/// that does not hold, or that another workload key signed, names no key of
/// this agent's and is left out.
fn vouched_keys(card: &Value, did: &str, now: u64) -> Vec<VouchedKey> {
    let Some(extensions) = card.get("extensions").and_then(Value::as_array) else {
        return Vec::new();
    };
    extensions
        .iter()
        .filter(|extension| {
            extension.get("uri").and_then(Value::as_str)
                == Some(arkavo_key_statement::CARD_EXTENSION)
        })
        .filter_map(|extension| extension.pointer("/data/statements")?.as_array())
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|compact| arkavo_key_statement::verify(compact, now).ok())
        .filter(|statement| statement.issuer == did)
        .map(|statement| VouchedKey {
            purpose: statement.purpose,
            key: statement.key,
            valid_until: statement.not_after,
        })
        .collect()
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
                did: keypair.public_key().to_did_key(),
                keys: vec![],
            })
        );
    }

    /// A statement in the card's key statements extension, signed by
    /// `workload` for `key`.
    fn statement(workload: &P256SigningKeypair, purpose: Purpose, key: String) -> String {
        let did = workload.public_key().to_did_key();
        let kid = format!("{did}#{}", did.trim_start_matches("did:key:"));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = arkavo_key_statement::KeyStatement {
            issuer: did,
            key,
            purpose,
            generation: 1,
            not_before: now - 60,
            not_after: now + 3600,
        };
        arkavo_key_statement::issue(&claims, &kid, |input| {
            Ok::<_, String>(workload.sign_p1363(input))
        })
        .unwrap()
    }

    #[spec("SRV-018")]
    #[tokio::test]
    async fn a_signed_card_lists_the_keys_its_identity_vouches_for() {
        use arkavo_crypto::AgentKeypair;
        use arkavo_protocol::types::AgentExtension;

        let workload = P256SigningKeypair::generate();
        let gossip = AgentKeypair::generate().public_key().to_did_key();
        let tdf = P256SigningKeypair::generate().public_key().to_did_key();
        let stranger = P256SigningKeypair::generate();
        let statements = vec![
            statement(&workload, Purpose::Gossip, gossip.clone()),
            statement(&workload, Purpose::Tdf, tdf.clone()),
            // Signed by another agent's workload key: not this agent's key.
            statement(
                &stranger,
                Purpose::Iroh,
                AgentKeypair::generate().public_key().to_did_key(),
            ),
            "not a statement".to_string(),
        ];
        let did = workload.public_key().to_did_key();
        let kid = format!("{did}#{}", did.trim_start_matches("did:key:"));
        let mut card = card();
        card.extensions.push(AgentExtension {
            uri: arkavo_key_statement::CARD_EXTENSION.to_string(),
            data: Some(serde_json::json!({ "statements": statements })),
        });
        card_signature::sign(&mut card, &kid, |input| {
            Ok::<_, String>(workload.sign_p1363(input))
        })
        .unwrap();

        let Some(CardIdentity::Verified { did: shown, keys }) =
            identity_of(serde_json::to_value(card).unwrap()).await
        else {
            panic!("the card verifies");
        };
        assert_eq!(shown, did);
        let listed: Vec<(Purpose, &str)> =
            keys.iter().map(|k| (k.purpose, k.key.as_str())).collect();
        assert_eq!(
            listed,
            [
                (Purpose::Gossip, gossip.as_str()),
                (Purpose::Tdf, tdf.as_str())
            ]
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
