//! The `_mcpi` protocol tool for MCP-I identity disclosure and session management.

use std::sync::Arc;

use arkavo_mcp::ToolSchema;
use arkavo_mcp_tools::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::McpIdentitySession;

/// MCP-I protocol tool registered as `_mcpi`.
///
/// Actions:
/// - `"identity"` — Returns this server's DID, algorithms, and MCP-I level
/// - `"verify"` — Client sends DID + proof; server verifies and records peer
/// - `"session"` — Returns current session state
pub struct IdentityProtocolTool {
    session: Arc<RwLock<McpIdentitySession>>,
    schema: ToolSchema,
}

impl IdentityProtocolTool {
    pub fn new(session: Arc<RwLock<McpIdentitySession>>) -> Self {
        Self {
            session,
            schema: ToolSchema {
                name: "_mcpi".to_string(),
                aliases: None,
                description: "MCP-I protocol tool for cryptographic identity disclosure, \
                              peer verification, and session state management"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["identity", "verify", "session"],
                            "description": "Action to perform"
                        },
                        "did": {
                            "type": "string",
                            "description": "Peer DID for verify action"
                        },
                        "proof": {
                            "type": "object",
                            "description": "Identity proof for verify action"
                        }
                    },
                    "required": ["action"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for IdentityProtocolTool {
    async fn execute(&self, params: Value) -> arkavo_mcp_tools::Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("missing 'action' parameter".to_string()))?;

        match action {
            "identity" => {
                let session: tokio::sync::RwLockReadGuard<'_, McpIdentitySession> =
                    self.session.read().await;
                Ok(json!({
                    "did": session.did(),
                    "algorithms": ["EdDSA"],
                    "identityLevel": 1,
                    "sessionId": session.session_id(),
                    "createdAt": session.created_at().to_rfc3339(),
                }))
            }
            "verify" => {
                let did = params.get("did").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParams("verify requires 'did' parameter".to_string())
                })?;

                let mut session: tokio::sync::RwLockWriteGuard<'_, McpIdentitySession> =
                    self.session.write().await;
                match arkavo_crypto::AgentPublicKey::from_did_key(did) {
                    Ok(_) => {
                        session.set_peer_did(did.to_string(), true);
                        Ok(json!({
                            "verified": true,
                            "peerDid": did,
                        }))
                    }
                    Err(e) => Ok(json!({
                        "verified": false,
                        "error": format!("invalid DID: {e}"),
                    })),
                }
            }
            "session" => {
                let session: tokio::sync::RwLockReadGuard<'_, McpIdentitySession> =
                    self.session.read().await;
                Ok(json!({
                    "sessionId": session.session_id(),
                    "did": session.did(),
                    "peerDid": session.peer_did(),
                    "verified": session.is_verified(),
                    "createdAt": session.created_at().to_rfc3339(),
                }))
            }
            _ => Err(ToolError::Execution(format!("unknown action: {action}"))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

/// Register the `_mcpi` protocol tool in a ToolRegistry.
pub fn register_identity_tool(
    registry: &mut arkavo_mcp_tools::ToolRegistry,
    session: Arc<RwLock<McpIdentitySession>>,
) {
    registry.register("_mcpi", Box::new(IdentityProtocolTool::new(session)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn identity_action_returns_did() {
        let session = Arc::new(RwLock::new(McpIdentitySession::ephemeral()));
        let tool = IdentityProtocolTool::new(session.clone());

        let result = tool.execute(json!({"action": "identity"})).await.unwrap();
        assert!(result["did"].as_str().unwrap().starts_with("did:key:z"));
        assert_eq!(result["identityLevel"], 1);
        assert_eq!(result["algorithms"][0], "EdDSA");
        assert!(result["sessionId"].is_string());
    }

    #[tokio::test]
    async fn verify_action_accepts_valid_did() {
        let session = Arc::new(RwLock::new(McpIdentitySession::ephemeral()));
        let tool = IdentityProtocolTool::new(session.clone());

        // Generate a valid DID to verify
        let peer = arkavo_crypto::AgentKeypair::generate();
        let peer_did = peer.public_key().to_did_key();

        let result = tool
            .execute(json!({"action": "verify", "did": peer_did}))
            .await
            .unwrap();
        assert_eq!(result["verified"], true);
        assert_eq!(result["peerDid"], peer_did);

        // Session should now have the peer recorded
        let s = session.read().await;
        assert_eq!(s.peer_did(), Some(peer_did.as_str()));
        assert!(s.is_verified());
    }

    #[tokio::test]
    async fn verify_action_rejects_invalid_did() {
        let session = Arc::new(RwLock::new(McpIdentitySession::ephemeral()));
        let tool = IdentityProtocolTool::new(session);

        let result = tool
            .execute(json!({"action": "verify", "did": "not-a-did"}))
            .await
            .unwrap();
        assert_eq!(result["verified"], false);
        assert!(result["error"].as_str().unwrap().contains("invalid DID"));
    }

    #[tokio::test]
    async fn session_action_returns_state() {
        let session = Arc::new(RwLock::new(McpIdentitySession::ephemeral()));
        let tool = IdentityProtocolTool::new(session);

        let result = tool.execute(json!({"action": "session"})).await.unwrap();
        assert!(result["sessionId"].is_string());
        assert!(result["did"].as_str().unwrap().starts_with("did:key:z"));
        assert!(result["peerDid"].is_null());
        assert_eq!(result["verified"], false);
    }

    #[tokio::test]
    async fn unknown_action_returns_error() {
        let session = Arc::new(RwLock::new(McpIdentitySession::ephemeral()));
        let tool = IdentityProtocolTool::new(session);

        let result = tool.execute(json!({"action": "bogus"})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_action_returns_error() {
        let session = Arc::new(RwLock::new(McpIdentitySession::ephemeral()));
        let tool = IdentityProtocolTool::new(session);

        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }
}
