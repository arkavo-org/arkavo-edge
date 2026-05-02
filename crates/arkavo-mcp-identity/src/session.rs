//! MCP-I session state: identity, proof generation, and peer tracking.

use std::collections::HashMap;

use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::IdentityError;
use crate::jws::{DetachedJws, canonical_json};

/// MCP-I Level 1 session managing identity and signed proofs.
pub struct McpIdentitySession {
    keypair: AgentKeypair,
    did: String,
    session_id: String,
    created_at: DateTime<Utc>,
    peer_did: Option<String>,
    verified: bool,
    reputation: HashMap<String, PeerReputation>,
}

/// In-memory reputation counters for a peer (L1: no persistence).
#[derive(Debug, Clone)]
pub struct PeerReputation {
    pub did: String,
    pub successful_verifications: u32,
    pub failed_verifications: u32,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

impl McpIdentitySession {
    /// Create a session from the persisted device keypair.
    pub fn from_device_keypair() -> Result<Self, IdentityError> {
        let key_bytes = arkavo_device_identity::keypair::get_keypair()
            .map_err(|e| IdentityError::DeviceIdentity(e.to_string()))?
            .ok_or_else(|| IdentityError::DeviceIdentity("no device keypair found".to_string()))?;

        let keypair = AgentKeypair::from_bytes(&key_bytes)?;
        let did = keypair.public_key().to_did_key();

        Ok(Self {
            keypair,
            did,
            session_id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            peer_did: None,
            verified: false,
            reputation: HashMap::new(),
        })
    }

    /// Create a session with a fresh ephemeral keypair (no persistence).
    pub fn ephemeral() -> Self {
        let keypair = AgentKeypair::generate();
        let did = keypair.public_key().to_did_key();
        Self {
            keypair,
            did,
            session_id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            peer_did: None,
            verified: false,
            reputation: HashMap::new(),
        }
    }

    /// Sign a tool response, returning the `_meta` value to inject.
    pub fn sign_response(&self, tool_name: &str, result: &Value) -> Value {
        let payload_obj = serde_json::json!({
            "toolName": tool_name,
            "result": result,
        });
        let payload_bytes = canonical_json(&payload_obj);

        let jws = DetachedJws::sign(&self.keypair, &self.did, &payload_bytes);

        serde_json::json!({
            "identityProof": {
                "type": "detached-jws-ed25519",
                "jws": jws.to_compact(),
                "did": self.did,
                "timestamp": Utc::now().to_rfc3339(),
                "nonce": Uuid::new_v4().to_string(),
            }
        })
    }

    /// Verify a peer's identity proof against a tool response payload.
    pub fn verify_peer_proof(
        &mut self,
        proof: &Value,
        tool_name: &str,
        result: &Value,
    ) -> Result<String, IdentityError> {
        let ip = proof
            .get("identityProof")
            .ok_or_else(|| IdentityError::Jws("missing identityProof".to_string()))?;

        let jws_str = ip
            .get("jws")
            .and_then(|v| v.as_str())
            .ok_or_else(|| IdentityError::Jws("missing jws field".to_string()))?;

        let peer_did = ip
            .get("did")
            .and_then(|v| v.as_str())
            .ok_or_else(|| IdentityError::Jws("missing did field".to_string()))?;

        let jws = DetachedJws::from_compact(jws_str)?;
        let public_key = AgentPublicKey::from_did_key(peer_did)?;

        let payload_obj = serde_json::json!({
            "toolName": tool_name,
            "result": result,
        });
        let payload_bytes = canonical_json(&payload_obj);

        let now = Utc::now();
        match jws.verify(&public_key, &payload_bytes) {
            Ok(()) => {
                let rep = self
                    .reputation
                    .entry(peer_did.to_string())
                    .or_insert_with(|| PeerReputation {
                        did: peer_did.to_string(),
                        successful_verifications: 0,
                        failed_verifications: 0,
                        first_seen: now,
                        last_seen: now,
                    });
                rep.successful_verifications += 1;
                rep.last_seen = now;
                Ok(peer_did.to_string())
            }
            Err(e) => {
                let rep = self
                    .reputation
                    .entry(peer_did.to_string())
                    .or_insert_with(|| PeerReputation {
                        did: peer_did.to_string(),
                        successful_verifications: 0,
                        failed_verifications: 0,
                        first_seen: now,
                        last_seen: now,
                    });
                rep.failed_verifications += 1;
                rep.last_seen = now;
                Err(e)
            }
        }
    }

    pub fn did(&self) -> &str {
        &self.did
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn peer_did(&self) -> Option<&str> {
        self.peer_did.as_deref()
    }

    pub fn set_peer_did(&mut self, did: String, verified: bool) {
        self.peer_did = Some(did);
        self.verified = verified;
    }

    pub fn is_verified(&self) -> bool {
        self.verified
    }

    pub fn reputation(&self, did: &str) -> Option<&PeerReputation> {
        self.reputation.get(did)
    }

    pub fn created_at(&self) -> &DateTime<Utc> {
        &self.created_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ephemeral_session_has_did() {
        let session = McpIdentitySession::ephemeral();
        assert!(session.did().starts_with("did:key:z"));
        assert!(!session.session_id().is_empty());
    }

    #[test]
    fn sign_response_produces_valid_proof() {
        let session = McpIdentitySession::ephemeral();
        let result = serde_json::json!({"status": "ok"});
        let meta = session.sign_response("test_tool", &result);

        let proof = &meta["identityProof"];
        assert_eq!(proof["type"], "detached-jws-ed25519");
        assert!(proof["jws"].as_str().unwrap().contains(".."));
        assert_eq!(proof["did"], session.did());
        assert!(proof["timestamp"].is_string());
        assert!(proof["nonce"].is_string());
    }

    #[test]
    fn verify_peer_proof_round_trip() {
        let server = McpIdentitySession::ephemeral();
        let result = serde_json::json!({"data": 42});
        let meta = server.sign_response("my_tool", &result);

        let mut client = McpIdentitySession::ephemeral();
        let peer_did = client.verify_peer_proof(&meta, "my_tool", &result).unwrap();
        assert_eq!(peer_did, server.did());

        let rep = client.reputation(server.did()).unwrap();
        assert_eq!(rep.successful_verifications, 1);
        assert_eq!(rep.failed_verifications, 0);
    }

    #[test]
    fn verify_peer_proof_rejects_tampered_result() {
        let server = McpIdentitySession::ephemeral();
        let result = serde_json::json!({"data": 42});
        let meta = server.sign_response("my_tool", &result);

        let tampered = serde_json::json!({"data": 99});
        let mut client = McpIdentitySession::ephemeral();
        assert!(
            client
                .verify_peer_proof(&meta, "my_tool", &tampered)
                .is_err()
        );

        let rep = client.reputation(server.did()).unwrap();
        assert_eq!(rep.successful_verifications, 0);
        assert_eq!(rep.failed_verifications, 1);
    }

    #[test]
    fn set_peer_did() {
        let mut session = McpIdentitySession::ephemeral();
        assert!(session.peer_did().is_none());
        assert!(!session.is_verified());

        session.set_peer_did("did:key:z6MkTest".to_string(), true);
        assert_eq!(session.peer_did(), Some("did:key:z6MkTest"));
        assert!(session.is_verified());
    }
}
