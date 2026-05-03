//! Apply a SwarmKit manifest to a pool of mesh agents.
//!
//! The orchestrator's "apply kit" flow:
//!
//! 1. Load manifest from disk (plain YAML/JSON or `.swarmkit.tdf`).
//! 2. For each [`RoleSpec`], pick the best mesh agent from the pool
//!    using [`RoleCapabilityMatcher`] (capability overlap + load tie-break).
//! 3. Build one [`AgentSpecializationBundle`] per assigned role:
//!    persona derived from the role spec, ARP overlay derived via
//!    `arkavo_swarmkit_runtime::derive::derive_arp_for_role`, API tokens
//!    pulled from the [`TokenVault`] for the tools the role uses.
//! 4. Wrap each bundle in TDF with a policy bound to the assigned
//!    agent's DID and ship it via [`BundleShipper`] (typically
//!    `agent.specialize` over A2A).
//! 5. Launch the SwarmFlight with `role_agent_bindings` populated and
//!    dispatch the first per-role task via the supplied
//!    `RoleTaskTransport`.
//!
//! The "what to do today" parts of this flow (LLM dispatch, multi-step
//! coordination) live further downstream — this module is the binding
//! step that takes identity-only agents and makes them
//! hyperspecialized.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arkavo_protocol::AgentSpecializationBundle;
use arkavo_protocol::agent_registry::AgentRegistry;
use arkavo_protocol::agent_specialization::{AgentPersona, McpToolGrant, RoleContext};
use arkavo_swarmkit::{Manifest, RoleSpec, parse_json, parse_yaml};
use arkavo_swarmkit_runtime::{
    DispatchHandle, KitPathKind, LaunchOptions, RoleTaskTransport, SwarmFlight, classify_kit_path,
    derive::DeriveOptions, derive::derive_arp_for_role,
};
use async_trait::async_trait;
use tracing::{info, warn};
use uuid::Uuid;

/// Errors returned by [`apply_kit`].
#[derive(Debug, thiserror::Error)]
pub enum ApplyKitError {
    #[error("read manifest at {path:?}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parse manifest at {path:?}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("path {path:?} has unsupported extension; expected .yaml/.yml/.json")]
    UnsupportedExtension { path: PathBuf },
    #[error(
        "loading .swarmkit.tdf manifests requires the orchestrator to be built with TDF support"
    )]
    TdfNotSupported { path: PathBuf },
    #[error("no agent in pool can fulfill role {role_id:?} (required: {required:?})")]
    RoleUncoverable {
        role_id: String,
        required: Vec<String>,
    },
    #[error("ship bundle to agent {agent_did:?} for role {role_id:?}: {message}")]
    Ship {
        role_id: String,
        agent_did: String,
        message: String,
    },
    #[error("launch SwarmFlight: {0}")]
    Launch(String),
    #[error("dispatch first task: {0}")]
    Dispatch(String),
}

/// Outcome of [`apply_kit`]: the launched flight plus the role→agent
/// assignment plan and the per-role first-task dispatch handles.
#[derive(Debug)]
pub struct AppliedKit {
    pub flight_id: Uuid,
    pub kit_id: String,
    pub kit_name: String,
    pub bindings: Vec<RoleBinding>,
    pub dispatch_handles: Vec<DispatchHandle>,
    pub flight: Arc<SwarmFlight>,
}

/// One row in the assignment plan.
#[derive(Debug, Clone)]
pub struct RoleBinding {
    pub role_id: String,
    pub role_type: String,
    pub agent_did: String,
    pub rationale: String,
}

/// Source of API tokens scoped to per-role tool grants.
///
/// In-memory implementation ([`InMemoryTokenVault`]) is supplied for
/// the demo; production wires an OS-keyring-backed impl.
#[async_trait]
pub trait TokenVault: Send + Sync {
    /// Fetch every token the role needs, keyed by token name. Returns an
    /// empty map when no tokens are required.
    async fn tokens_for_role(&self, role: &RoleSpec) -> HashMap<String, String>;
}

/// In-memory `TokenVault`. The map is `(server, token_name) → value`,
/// and the lookup grants a token only when the role declares a grant
/// for that server.
#[derive(Default)]
pub struct InMemoryTokenVault {
    tokens: HashMap<String, HashMap<String, String>>,
}

impl InMemoryTokenVault {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, server: &str, token_name: &str, value: &str) {
        self.tokens
            .entry(server.to_string())
            .or_default()
            .insert(token_name.to_string(), value.to_string());
    }
}

#[async_trait]
impl TokenVault for InMemoryTokenVault {
    async fn tokens_for_role(&self, role: &RoleSpec) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for grant in &role.mcp_tools {
            if let Some(per_server) = self.tokens.get(&grant.server) {
                for (k, v) in per_server.iter() {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
        out
    }
}

/// Encrypts a built bundle for a recipient and returns the bytes that
/// the orchestrator will ship over A2A. Trait so production wires
/// `arkavo-tdf::OpenTdfService` and tests inject a fake.
#[async_trait]
pub trait BundleEncryptor: Send + Sync {
    async fn wrap(
        &self,
        bundle: &AgentSpecializationBundle,
        recipient_did: &str,
    ) -> Result<Vec<u8>, String>;
}

/// Ships wrapped bundles to receiving agents — typically via
/// `agent.specialize` A2A RPC. Trait keeps the apply pipeline
/// transport-agnostic.
#[async_trait]
pub trait BundleShipper: Send + Sync {
    async fn ship(&self, agent_did: &str, tdf_bytes: &[u8]) -> Result<(), String>;
}

/// Pluggable capability matcher. Reads a role's `skills` and
/// `mcp_tools` and ranks the agent pool. Default impl uses
/// `AgentRegistry::find_best_agent`.
pub struct RoleCapabilityMatcher {
    registry: Arc<AgentRegistry>,
}

impl RoleCapabilityMatcher {
    pub fn new(registry: Arc<AgentRegistry>) -> Self {
        Self { registry }
    }

    /// Required-capability strings derived from a role: skill ids first
    /// (most specific), then mcp tool servers (broader). Matchers walk
    /// this list in order, taking the first capability that resolves
    /// to an available agent.
    fn required_capabilities(role: &RoleSpec) -> Vec<String> {
        let mut caps = Vec::new();
        for skill in &role.skills {
            caps.push(skill.id.clone());
        }
        for grant in &role.mcp_tools {
            caps.push(grant.server.clone());
        }
        caps.dedup();
        caps
    }

    /// Pick the best agent for a role from the pool. Returns
    /// `(agent_did, rationale)` or `None` when no agent in the pool
    /// covers any of the role's required capabilities.
    pub async fn match_role(&self, role: &RoleSpec) -> Option<(String, String)> {
        let required = Self::required_capabilities(role);
        if required.is_empty() {
            // No capability constraint at all → pick any available
            // agent. Caller decides whether that's acceptable.
            return None;
        }
        for capability in &required {
            if let Some(agent_id) = self.registry.find_best_agent(capability).await {
                return Some((
                    agent_id,
                    format!(
                        "matched on capability {capability:?} from role {role_id:?}",
                        role_id = role.id,
                    ),
                ));
            }
        }
        None
    }
}

/// Apply a SwarmKit manifest end-to-end. See the module docs for the
/// step-by-step flow.
pub async fn apply_kit(
    kit_path: &Path,
    matcher: &RoleCapabilityMatcher,
    vault: &dyn TokenVault,
    encryptor: &dyn BundleEncryptor,
    shipper: &dyn BundleShipper,
    transport: &dyn RoleTaskTransport,
) -> Result<AppliedKit, ApplyKitError> {
    let manifest = load_manifest(kit_path)?;
    info!(
        kit_name = %manifest.kit.name,
        role_count = manifest.roles.len(),
        "apply_kit: manifest loaded"
    );

    // 1. Capability matching per role.
    let mut bindings = Vec::with_capacity(manifest.roles.len());
    for role in &manifest.roles {
        let (agent_did, rationale) =
            matcher
                .match_role(role)
                .await
                .ok_or_else(|| ApplyKitError::RoleUncoverable {
                    role_id: role.id.clone(),
                    required: RoleCapabilityMatcher::required_capabilities(role),
                })?;
        bindings.push(RoleBinding {
            role_id: role.id.clone(),
            role_type: role.role_type.clone(),
            agent_did,
            rationale,
        });
    }

    // 2. Pre-allocate flight_id so role_context tracks it before launch.
    let flight_id = Uuid::new_v4();

    // 3. Build + ship bundles.
    let role_count = manifest.roles.len();
    let derive_opts = DeriveOptions::default();
    for (role, binding) in manifest.roles.iter().zip(bindings.iter()) {
        let bundle = build_bundle(
            &manifest,
            role,
            binding,
            flight_id,
            role_count,
            derive_opts,
            vault,
        )
        .await;
        let tdf_bytes = encryptor
            .wrap(&bundle, &binding.agent_did)
            .await
            .map_err(|message| ApplyKitError::Ship {
                role_id: binding.role_id.clone(),
                agent_did: binding.agent_did.clone(),
                message: format!("wrap: {message}"),
            })?;
        shipper
            .ship(&binding.agent_did, &tdf_bytes)
            .await
            .map_err(|message| ApplyKitError::Ship {
                role_id: binding.role_id.clone(),
                agent_did: binding.agent_did.clone(),
                message,
            })?;
    }

    // 4. Launch with bindings populated.
    let mut role_agent_bindings = HashMap::new();
    for b in &bindings {
        role_agent_bindings.insert(b.role_id.clone(), b.agent_did.clone());
    }
    let flight = SwarmFlight::launch(
        &manifest,
        LaunchOptions {
            flight_id: Some(flight_id),
            role_agent_bindings,
            ..LaunchOptions::default()
        },
    )
    .map_err(|e| ApplyKitError::Launch(e.to_string()))?;
    let flight = Arc::new(flight);

    // 5. Dispatch first task per role via the supplied transport.
    let dispatch_handles = flight
        .dispatch_initial_tasks(transport)
        .await
        .map_err(|e| ApplyKitError::Dispatch(e.to_string()))?;

    info!(
        kit_name = %manifest.kit.name,
        flight_id = %flight_id,
        bound_roles = bindings.len(),
        first_task_dispatches = dispatch_handles.len(),
        "apply_kit: complete"
    );

    Ok(AppliedKit {
        flight_id,
        kit_id: manifest.kit.id.clone(),
        kit_name: manifest.kit.name.clone(),
        bindings,
        dispatch_handles,
        flight,
    })
}

fn load_manifest(path: &Path) -> Result<Manifest, ApplyKitError> {
    match classify_kit_path(path) {
        KitPathKind::Yaml => {
            let raw = std::fs::read_to_string(path).map_err(|source| ApplyKitError::Read {
                path: path.to_path_buf(),
                source,
            })?;
            parse_yaml(&raw).map_err(|e| ApplyKitError::Parse {
                path: path.to_path_buf(),
                message: e.to_string(),
            })
        }
        KitPathKind::Json => {
            let raw = std::fs::read_to_string(path).map_err(|source| ApplyKitError::Read {
                path: path.to_path_buf(),
                source,
            })?;
            parse_json(&raw).map_err(|e| ApplyKitError::Parse {
                path: path.to_path_buf(),
                message: e.to_string(),
            })
        }
        KitPathKind::Tdf => Err(ApplyKitError::TdfNotSupported {
            path: path.to_path_buf(),
        }),
        KitPathKind::Unknown => Err(ApplyKitError::UnsupportedExtension {
            path: path.to_path_buf(),
        }),
    }
}

async fn build_bundle(
    manifest: &Manifest,
    role: &RoleSpec,
    binding: &RoleBinding,
    flight_id: Uuid,
    role_count: usize,
    derive_opts: DeriveOptions,
    vault: &dyn TokenVault,
) -> AgentSpecializationBundle {
    let model_label = role
        .agent_provisioning
        .model
        .as_ref()
        .map(|m| match (m.size.as_deref(), m.backend.as_deref()) {
            (Some(size), Some(backend)) => format!("{}-{} ({})", m.family, size, backend),
            (Some(size), None) => format!("{}-{}", m.family, size),
            _ => m.family.clone(),
        })
        .unwrap_or_else(|| "(unspecified)".to_string());

    let persona = AgentPersona {
        name: binding.agent_did.clone(),
        purpose: role
            .description
            .clone()
            .unwrap_or_else(|| format!("Specialist for {} in {}", role.id, manifest.kit.name)),
        model: model_label,
        mcp_tools: role
            .mcp_tools
            .iter()
            .map(|g| McpToolGrant {
                server: g.server.clone(),
                tools: g.tools.clone(),
            })
            .collect(),
    };

    let arp_overlay = derive_arp_for_role(
        role,
        &manifest.constraints.global_budget,
        role_count,
        derive_opts,
    );

    let role_context = RoleContext {
        kit_id: manifest.kit.id.clone(),
        flight_id: flight_id.to_string(),
        role_id: role.id.clone(),
        role_type: role.role_type.clone(),
        deliverable_schema: manifest
            .deliverables
            .first()
            .map(|d| serde_json::json!({"name": d.name, "type": format!("{:?}", d.payload_type)}))
            .unwrap_or_else(|| serde_json::json!({})),
        handoff_targets: role.handoffs.iter().map(|h| h.to.clone()).collect(),
    };

    let api_tokens = vault.tokens_for_role(role).await;
    if !api_tokens.is_empty() {
        info!(
            role = %role.id,
            agent = %binding.agent_did,
            token_count = api_tokens.len(),
            "supplying API tokens for role"
        );
    } else if !role.mcp_tools.is_empty() {
        warn!(
            role = %role.id,
            agent = %binding.agent_did,
            "role has MCP tool grants but vault returned no tokens"
        );
    }

    AgentSpecializationBundle {
        persona,
        api_tokens,
        arp_overlay,
        role_context,
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_swarmkit_runtime::RoleTaskEnvelope;
    use std::collections::HashMap as StdHashMap;
    use std::pin::Pin;
    use std::sync::Mutex;
    use tokio::sync::Mutex as AsyncMutex;

    const KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "test-kit"
  version: "0.1.0"
  authors: [{did: "did:web:example.com"}]
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "exercise apply_kit"
  success_criteria: ["done"]
inputs: []
deliverables: [{name: "out", type: "json"}]
roles:
  - id: "analyst"
    role_type: "asset_analyst"
    agent_provisioning:
      model: {family: "gemma-4", size: "9B"}
    skills:
      - {id: "summarize", version: "0.1.0", source: "inline"}
    mcp_tools:
      - {server: "asset-store", tools: ["read", "describe"], auth: "delegated"}
  - id: "copy"
    role_type: "platform_copy"
    agent_provisioning:
      model: {family: "qwen3", size: "7B"}
    skills:
      - {id: "write", version: "0.1.0", source: "inline"}
    mcp_tools:
      - {server: "social-publisher", tools: ["post"], auth: "delegated"}
coordination:
  topology: "pipeline"
  routing: {strategy: "static"}
constraints:
  global_budget: {max_wallclock_seconds: 60, max_total_tokens: 8000, max_cost_usd: 0.10}
  data_classifications: ["public"]
  network: {egress_allowed: false, egress_allowlist: []}
completion:
  rules: ["all deliverables present"]
  on_failure: "abort"
  max_retries: 0
provenance:
  signatures: [{signer_did: "did:web:example.com", algorithm: "ed25519", signature: "AAA"}]
"#;

    #[derive(Default)]
    struct StubEncryptor {
        wraps: AsyncMutex<Vec<(String, AgentSpecializationBundle)>>,
    }

    #[async_trait]
    impl BundleEncryptor for StubEncryptor {
        async fn wrap(
            &self,
            bundle: &AgentSpecializationBundle,
            recipient_did: &str,
        ) -> Result<Vec<u8>, String> {
            self.wraps
                .lock()
                .await
                .push((recipient_did.to_string(), bundle.clone()));
            // Stub "ciphertext" is just JSON of the bundle.
            serde_json::to_vec(bundle).map_err(|e| e.to_string())
        }
    }

    #[derive(Default)]
    struct StubShipper {
        sent: AsyncMutex<Vec<(String, Vec<u8>)>>,
    }

    #[async_trait]
    impl BundleShipper for StubShipper {
        async fn ship(&self, agent_did: &str, tdf_bytes: &[u8]) -> Result<(), String> {
            self.sent
                .lock()
                .await
                .push((agent_did.to_string(), tdf_bytes.to_vec()));
            Ok(())
        }
    }

    #[derive(Default)]
    struct StubTransport {
        dispatched: Mutex<Vec<RoleTaskEnvelope>>,
    }

    impl RoleTaskTransport for StubTransport {
        fn dispatch<'a>(
            &'a self,
            envelope: RoleTaskEnvelope,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
            Box::pin(async move {
                self.dispatched.lock().unwrap().push(envelope);
                Ok(())
            })
        }
    }

    async fn pool_with_capabilities(agents: &[(&str, &[&str])]) -> Arc<AgentRegistry> {
        let registry = Arc::new(AgentRegistry::new());
        for (agent_id, caps) in agents {
            registry
                .register_agent(
                    (*agent_id).to_string(),
                    (*agent_id).to_string(),
                    "test agent".to_string(),
                    caps.iter().map(|s| (*s).to_string()).collect(),
                    None,
                    StdHashMap::new(),
                    None,
                )
                .await
                .expect("register");
        }
        registry
    }

    #[tokio::test]
    async fn match_role_picks_capability_overlap() {
        let pool = pool_with_capabilities(&[
            ("did:web:agent-a", &["summarize"]),
            ("did:web:agent-b", &["write"]),
        ])
        .await;
        let matcher = RoleCapabilityMatcher::new(pool);
        let manifest = parse_yaml(KIT).expect("manifest");

        let analyst = &manifest.roles[0];
        let (agent, _) = matcher.match_role(analyst).await.expect("match");
        assert_eq!(agent, "did:web:agent-a");

        let copy = &manifest.roles[1];
        let (agent, _) = matcher.match_role(copy).await.expect("match");
        assert_eq!(agent, "did:web:agent-b");
    }

    #[tokio::test]
    async fn match_role_returns_none_when_uncoverable() {
        let pool = pool_with_capabilities(&[("did:web:agent-z", &["unrelated"])]).await;
        let matcher = RoleCapabilityMatcher::new(pool);
        let manifest = parse_yaml(KIT).expect("manifest");
        assert!(matcher.match_role(&manifest.roles[0]).await.is_none());
    }

    #[tokio::test]
    async fn apply_kit_returns_full_assignment_plan() {
        let pool = pool_with_capabilities(&[
            ("did:web:agent-a", &["summarize", "asset-store"]),
            ("did:web:agent-b", &["write", "social-publisher"]),
        ])
        .await;

        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("test.swarmkit.yaml");
        std::fs::write(&path, KIT).expect("write");

        let matcher = RoleCapabilityMatcher::new(pool);
        let mut vault = InMemoryTokenVault::new();
        vault.insert("asset-store", "ASSET_TOKEN", "tok-asset");
        vault.insert("social-publisher", "SOCIAL_TOKEN", "tok-social");
        let encryptor = StubEncryptor::default();
        let shipper = StubShipper::default();
        let transport = StubTransport::default();

        let applied = apply_kit(&path, &matcher, &vault, &encryptor, &shipper, &transport)
            .await
            .expect("apply");

        assert_eq!(applied.bindings.len(), 2);
        assert_eq!(applied.bindings[0].role_id, "analyst");
        assert_eq!(applied.bindings[0].agent_did, "did:web:agent-a");
        assert_eq!(applied.bindings[1].role_id, "copy");
        assert_eq!(applied.bindings[1].agent_did, "did:web:agent-b");

        let wraps = encryptor.wraps.lock().await;
        assert_eq!(wraps.len(), 2);
        assert_eq!(wraps[0].0, "did:web:agent-a");
        // Bundle for agent-a carries asset-store grant + asset token only.
        assert!(wraps[0].1.api_tokens.contains_key("ASSET_TOKEN"));
        assert!(!wraps[0].1.api_tokens.contains_key("SOCIAL_TOKEN"));
        // role_context populated.
        assert_eq!(wraps[0].1.role_context.role_id, "analyst");
        assert_eq!(
            wraps[0].1.role_context.flight_id,
            applied.flight_id.to_string()
        );

        let sent = shipper.sent.lock().await;
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].0, "did:web:agent-a");

        // First task dispatched per role.
        assert_eq!(applied.dispatch_handles.len(), 2);
        let dispatched = transport.dispatched.lock().unwrap();
        assert_eq!(dispatched.len(), 2);
        assert_eq!(dispatched[0].role_id, "analyst");
        assert_eq!(dispatched[0].agent_did, "did:web:agent-a");
    }

    #[tokio::test]
    async fn apply_kit_errors_when_role_uncoverable() {
        let pool = pool_with_capabilities(&[("did:web:agent-z", &["unrelated"])]).await;

        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("test.swarmkit.yaml");
        std::fs::write(&path, KIT).expect("write");

        let matcher = RoleCapabilityMatcher::new(pool);
        let vault = InMemoryTokenVault::new();
        let encryptor = StubEncryptor::default();
        let shipper = StubShipper::default();
        let transport = StubTransport::default();

        let err = apply_kit(&path, &matcher, &vault, &encryptor, &shipper, &transport)
            .await
            .expect_err("uncoverable");

        match err {
            ApplyKitError::RoleUncoverable { role_id, .. } => assert_eq!(role_id, "analyst"),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn apply_kit_rejects_tdf_path() {
        let pool = Arc::new(AgentRegistry::new());
        let matcher = RoleCapabilityMatcher::new(pool);
        let vault = InMemoryTokenVault::new();
        let encryptor = StubEncryptor::default();
        let shipper = StubShipper::default();
        let transport = StubTransport::default();

        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("kit.swarmkit.tdf");
        std::fs::write(&path, b"unused").expect("write");

        let err = apply_kit(&path, &matcher, &vault, &encryptor, &shipper, &transport)
            .await
            .expect_err("tdf rejected");
        assert!(matches!(err, ApplyKitError::TdfNotSupported { .. }));
    }

    #[tokio::test]
    async fn token_vault_yields_only_relevant_tokens() {
        let mut vault = InMemoryTokenVault::new();
        vault.insert("asset-store", "ASSET_TOKEN", "v");
        vault.insert("social-publisher", "SOCIAL_TOKEN", "v");
        let manifest = parse_yaml(KIT).expect("manifest");
        let analyst = &manifest.roles[0];

        let tokens = vault.tokens_for_role(analyst).await;
        assert_eq!(tokens.len(), 1);
        assert!(tokens.contains_key("ASSET_TOKEN"));
    }
}
