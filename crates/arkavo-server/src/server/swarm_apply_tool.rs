//! Agent-loop tool: apply a SwarmKit manifest to the mesh.
//!
//! This is the agent-driven entry point for SwarmKit specialization — the
//! human gives a task, the orchestrator agent picks the right manifest and
//! calls this tool (rather than a human running a CLI). It composes the
//! sender-side apply pipeline over the mesh's discovered agents:
//!
//! 1. match each role to an available agent ([`RoleCapabilityMatcher`]);
//! 2. wrap a TDF specialization bundle bound to that agent
//!    ([`TdfBundleEncryptor`], KAS-backed);
//! 3. stage it on the Iroh data plane and signal the ticket
//!    ([`IrohBundleShipper`]);
//! 4. dispatch each role's first task ([`MeshRoleTaskTransport`]),
//!    scoping the `repo_maintainer` role to the triggering org/repo
//!    ([`RepoScope`]).
//!
//! The Iroh node is the server's own long-lived node, so staged bundles stay
//! fetchable for the lifetime of the agent process. Gated on `kas` + `iroh`:
//! without a KAS-backed encryptor the pipeline must not ship token-bearing
//! bundles in the clear, and without Iroh there is no data plane to stage on.

use std::path::Path;
use std::sync::Arc;

use arkavo_mcp_mesh::MeshToolsState;
use arkavo_mcp_tools::server::{Tool, error_response};
use arkavo_mcp_tools::{Result, ToolSchema};
use arkavo_orchestrator::{
    EnvTokenVault, IrohBundleShipper, MeshRoleTaskTransport, RepoScope, RoleCapabilityMatcher,
    TdfBundleEncryptor, apply_kit,
};
use arkavo_tdf_iroh::{IrohNode, IrohTransport};
use async_trait::async_trait;
use serde_json::{Value, json};

/// Default KAS endpoint when `ARKAVO_KAS_URL` is unset. Matches
/// `arkavo_tdf::OpenTdfConfig`'s default so wrap and rewrap agree.
const DEFAULT_KAS_URL: &str = "https://kas.arkavo.net";

/// Agent-callable tool that applies a SwarmKit manifest to the mesh.
pub(super) struct SwarmApplyTool {
    schema: ToolSchema,
    mesh_state: Arc<MeshToolsState>,
    iroh_node: Arc<IrohNode>,
    kas_url: String,
    requester_did: String,
}

impl SwarmApplyTool {
    /// `requester_did` is the orchestrating agent's own id (carried in the
    /// `agent.specialize` request); `kas_url` is read from `ARKAVO_KAS_URL`,
    /// falling back to [`DEFAULT_KAS_URL`].
    pub(super) fn new(
        mesh_state: Arc<MeshToolsState>,
        iroh_node: Arc<IrohNode>,
        requester_did: String,
    ) -> Self {
        let kas_url =
            std::env::var("ARKAVO_KAS_URL").unwrap_or_else(|_| DEFAULT_KAS_URL.to_string());
        Self {
            schema: ToolSchema {
                name: "apply_swarmkit".to_string(),
                aliases: None,
                description: "Apply a SwarmKit manifest to the mesh: match each role to a \
                    discovered agent, wrap a TDF specialization bundle bound to that agent, \
                    ship it over the Iroh data plane, and dispatch each role's first task. Use \
                    when a task needs a coordinated team of specialist agents described by a \
                    .swarmkit.yaml manifest."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "manifest_path": {
                            "type": "string",
                            "description": "Filesystem path to the .swarmkit.yaml manifest to apply."
                        },
                        "org": {
                            "type": "string",
                            "description": "GitHub org/owner the swarm operates on; pins the repo_maintainer role to it."
                        },
                        "repo": {
                            "type": "string",
                            "description": "Optional repository name scoping the repo_maintainer role. Defaults to org-level."
                        }
                    },
                    "required": ["manifest_path", "org"]
                }),
            },
            mesh_state,
            iroh_node,
            kas_url,
            requester_did,
        }
    }
}

#[async_trait]
impl Tool for SwarmApplyTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let Some(manifest_path) = params.get("manifest_path").and_then(Value::as_str) else {
            return Ok(error_response("manifest_path is required"));
        };
        let Some(org) = params.get("org").and_then(Value::as_str) else {
            return Ok(error_response("org is required"));
        };
        let repo = params.get("repo").and_then(Value::as_str).unwrap_or("");

        // Refresh discovery so role matching sees currently-available agents.
        self.mesh_state.discover_peers().await;

        let registry = Arc::clone(&self.mesh_state.registry);
        let matcher = RoleCapabilityMatcher::new(Arc::clone(&registry));
        let vault = EnvTokenVault;
        let scope = RepoScope::new(org, repo);
        let encryptor = TdfBundleEncryptor::new(self.kas_url.clone());
        let iroh = Arc::new(IrohTransport::new(Arc::clone(&self.iroh_node)));
        let shipper =
            IrohBundleShipper::new(iroh, Arc::clone(&registry), self.requester_did.clone());
        let transport = MeshRoleTaskTransport::new(registry);

        match apply_kit(
            Path::new(manifest_path),
            &matcher,
            &vault,
            &encryptor,
            &shipper,
            &transport,
            Some(&scope),
        )
        .await
        {
            Ok(applied) => Ok(json!({
                "success": true,
                "flight_id": applied.flight_id.to_string(),
                "kit_name": applied.kit_name,
                "roles_bound": applied.bindings.len(),
                "tasks_dispatched": applied.dispatch_handles.len(),
                "bindings": applied.bindings.iter().map(|b| json!({
                    "role_id": b.role_id,
                    "role_type": b.role_type,
                    "agent_did": b.agent_did,
                    "rationale": b.rationale,
                })).collect::<Vec<_>>(),
            })),
            Err(e) => Ok(error_response(&format!("apply_kit failed: {e}"))),
        }
    }
}
