//! Production mesh transports for the SwarmKit apply pipeline.
//!
//! Two concrete trait impls that compose existing infra — they add no
//! new protocol, crypto, or discovery:
//!
//! * [`IrohBundleShipper`] (impl [`BundleShipper`]) stages the
//!   already-TDF-wrapped bundle on the Iroh data plane, resolves the
//!   target agent's A2A address from the [`AgentRegistry`], and signals
//!   the resulting ticket via the `agent.specialize` A2A RPC.
//! * [`MeshRoleTaskTransport`] (impl [`RoleTaskTransport`]) delivers each
//!   role's first task to its bound agent via the `message/send` A2A RPC
//!   — the arrival of that task is the role's start signal.
//!
//! The bundle is a sizeable blob, so it travels over Iroh per the channel
//! rule; only the small ticket rides the A2A control plane.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use arkavo_protocol::agent_registry::AgentRegistry;
use arkavo_protocol::transport::TlsConfig;
use arkavo_protocol::types::{
    AgentSpecializeRequest, AgentSpecializeResponse, Message, MessagePart, MessageSendRequest,
    MessageSendResponse,
};
use arkavo_protocol::{
    A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, HttpTransport, TransportConfig,
};
use arkavo_swarmkit_runtime::{RoleTaskEnvelope, RoleTaskTransport};
use arkavo_tdf_iroh::IrohTransport;

use crate::swarmkit_apply::BundleShipper;

/// Short-lived A2A transport config matching the mesh send idiom used by
/// `SendTaskTool` and `task_executor::mesh_strategy`.
fn a2a_transport_config() -> TransportConfig {
    TransportConfig {
        timeout_ms: 60_000,
        max_retries: 2,
        tls_config: TlsConfig {
            require_tls: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Resolve a bound agent DID to its A2A endpoint URL via the registry.
async fn resolve_agent_address(
    registry: &AgentRegistry,
    agent_did: &str,
) -> Result<String, String> {
    let info = registry
        .get_agent_info(agent_did)
        .await
        .ok_or_else(|| format!("agent {agent_did:?} not found in registry"))?;
    info.address
        .ok_or_else(|| format!("agent {agent_did:?} has no A2A address configured"))
}

/// Connect a fresh `HttpTransport` to the agent's endpoint.
async fn connect_to(agent_did: &str, address: &str) -> Result<HttpTransport, String> {
    let transport =
        HttpTransport::new(a2a_transport_config()).map_err(|e| format!("create transport: {e}"))?;
    let endpoint = A2aEndpoint {
        url: address.to_string(),
        agent_id: agent_did.to_string(),
        public_key: None,
    };
    transport
        .connect(&endpoint)
        .await
        .map_err(|e| format!("connect to {agent_did:?} at {address}: {e}"))?;
    Ok(transport)
}

/// Ships TDF-wrapped specialization bundles over the mesh: stage on the
/// Iroh data plane, signal the ticket over A2A `agent.specialize`.
pub struct IrohBundleShipper {
    iroh: Arc<IrohTransport>,
    registry: Arc<AgentRegistry>,
    requester_did: String,
}

impl IrohBundleShipper {
    pub fn new(
        iroh: Arc<IrohTransport>,
        registry: Arc<AgentRegistry>,
        requester_did: String,
    ) -> Self {
        Self {
            iroh,
            registry,
            requester_did,
        }
    }
}

#[async_trait::async_trait]
impl BundleShipper for IrohBundleShipper {
    async fn ship(&self, agent_did: &str, tdf_bytes: &[u8]) -> Result<(), String> {
        // 1. Stage the already-wrapped bundle on the data plane → ticket.
        let ticket = self
            .iroh
            .stage_bytes(tdf_bytes)
            .await
            .map_err(|e| format!("stage bundle on Iroh: {e}"))?;
        let ticket_str = ticket.to_string();

        // 2. Resolve the target agent's A2A address via mesh discovery.
        let address = resolve_agent_address(&self.registry, agent_did).await?;
        let transport = connect_to(agent_did, &address).await?;

        // 3. Signal the ticket over A2A. The bundle bytes never traverse
        //    the control plane — only the ticket does.
        // STRUCTURAL DEBT (a2a-realignment DEC-4): this `agent.specialize`
        // JSON-RPC call on the current A2A stack is replaced post-DEC-4 by a
        // `SendMessage` + TDF `Part` over `arkavo-config-transport`. The Iroh
        // staging + ticket survive the realignment; only this send migrates.
        let request = AgentSpecializeRequest {
            requester_id: self.requester_did.clone(),
            encrypted_bundle: String::new(),
            task_context: None,
            session_id: None,
            ticket: Some(ticket_str),
        };
        let rpc = A2aRequest::new("agent.specialize", serde_json::json!([request]));
        let response = transport
            .send_request(rpc)
            .await
            .map_err(|e| format!("send agent.specialize to {agent_did:?}: {e}"))?;
        let _ = transport.close().await;

        match response {
            A2aResponse::Success { result, .. } => {
                let resp: AgentSpecializeResponse = serde_json::from_value(result)
                    .map_err(|e| format!("parse agent.specialize response: {e}"))?;
                if resp.accepted {
                    Ok(())
                } else {
                    Err(format!(
                        "agent {agent_did:?} rejected specialization: {}",
                        resp.message.unwrap_or_default()
                    ))
                }
            }
            A2aResponse::Error { error, .. } => Err(format!(
                "agent.specialize rejected by {agent_did:?}: {} {}",
                error.code, error.message
            )),
        }
    }
}

/// Dispatches each role's first task to its bound agent over A2A
/// `message/send` — the task's arrival is the role's start signal.
pub struct MeshRoleTaskTransport {
    registry: Arc<AgentRegistry>,
}

impl MeshRoleTaskTransport {
    pub fn new(registry: Arc<AgentRegistry>) -> Self {
        Self { registry }
    }

    async fn dispatch_inner(&self, envelope: RoleTaskEnvelope) -> Result<(), String> {
        let address = resolve_agent_address(&self.registry, &envelope.agent_did).await?;
        let transport = connect_to(&envelope.agent_did, &address).await?;

        let message = Message {
            parts: vec![MessagePart::Text {
                content: envelope.task.clone(),
            }],
            metadata: Some(serde_json::json!({
                "source": "swarmkit_orchestrator",
                "flight_id": envelope.flight_id.to_string(),
                "role_id": envelope.role_id,
                "role_type": envelope.role_type,
            })),
        };
        let send_request = MessageSendRequest {
            message,
            task_id: None,
        };
        let rpc = A2aRequest::new("message/send", serde_json::json!([send_request]));
        let response = transport
            .send_request(rpc)
            .await
            .map_err(|e| format!("dispatch role task to {:?}: {e}", envelope.agent_did))?;
        let _ = transport.close().await;

        match response {
            A2aResponse::Success { result, .. } => {
                let _: MessageSendResponse = serde_json::from_value(result)
                    .map_err(|e| format!("parse message/send response: {e}"))?;
                Ok(())
            }
            A2aResponse::Error { error, .. } => Err(format!(
                "role task rejected by {:?}: {} {}",
                envelope.agent_did, error.code, error.message
            )),
        }
    }
}

impl RoleTaskTransport for MeshRoleTaskTransport {
    fn dispatch<'a>(
        &'a self,
        envelope: RoleTaskEnvelope,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move { self.dispatch_inner(envelope).await })
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_tdf_iroh::IrohNode;
    use std::collections::HashMap;

    async fn registry_with(agent_id: &str, address: Option<&str>) -> Arc<AgentRegistry> {
        let reg = Arc::new(AgentRegistry::new());
        reg.register_agent(
            agent_id.to_string(),
            agent_id.to_string(),
            "test agent".to_string(),
            vec!["asset-store".to_string()],
            None,
            HashMap::new(),
            address.map(|a| a.to_string()),
        )
        .await
        .expect("register");
        reg
    }

    #[tokio::test]
    async fn ship_errors_when_agent_has_no_address() {
        let node = IrohNode::memory().await.unwrap();
        let transport = Arc::new(IrohTransport::new(node));
        let reg = registry_with("did:web:agent-a", None).await;
        let shipper = IrohBundleShipper::new(transport, reg, "did:web:orch".into());
        let err = shipper
            .ship("did:web:agent-a", b"tdf-bytes")
            .await
            .expect_err("no address");
        assert!(err.contains("address"), "got: {err}");
    }

    #[tokio::test]
    async fn ship_errors_when_agent_unknown() {
        let node = IrohNode::memory().await.unwrap();
        let transport = Arc::new(IrohTransport::new(node));
        let reg = Arc::new(AgentRegistry::new());
        let shipper = IrohBundleShipper::new(transport, reg, "did:web:orch".into());
        let err = shipper
            .ship("did:web:nope", b"tdf-bytes")
            .await
            .expect_err("unknown agent");
        assert!(
            err.contains("not found") || err.contains("address"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn ship_stages_then_fails_only_on_unreachable_send() {
        // A bogus but well-formed address: staging + resolution succeed,
        // the A2A send is the only thing that fails — proving our glue ran.
        let node = IrohNode::memory().await.unwrap();
        let transport = Arc::new(IrohTransport::new(node));
        let reg = registry_with("did:web:agent-a", Some("http://127.0.0.1:1/")).await;
        let shipper = IrohBundleShipper::new(transport, reg, "did:web:orch".into());
        let err = shipper
            .ship("did:web:agent-a", b"tdf-bytes")
            .await
            .expect_err("send to dead endpoint fails");
        // Must NOT be a staging/resolution error.
        assert!(!err.contains("not found"));
        assert!(
            err.to_lowercase().contains("specialize")
                || err.to_lowercase().contains("send")
                || err.to_lowercase().contains("connect"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn dispatch_errors_when_agent_has_no_address() {
        let reg = registry_with("did:web:agent-a", None).await;
        let transport = MeshRoleTaskTransport::new(reg);
        let env = RoleTaskEnvelope {
            flight_id: uuid::Uuid::new_v4(),
            role_id: "reviewer".into(),
            role_type: "critic".into(),
            agent_did: "did:web:agent-a".into(),
            task: "review the PR".into(),
        };
        let err = transport.dispatch(env).await.expect_err("no address");
        assert!(err.contains("address"), "got: {err}");
    }
}
