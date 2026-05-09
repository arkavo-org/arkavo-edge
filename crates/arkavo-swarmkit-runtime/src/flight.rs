//! SwarmFlight — a launched SwarmKit with one ArpRuntime per role.
//!
//! The runtime is a logical orchestrator. It does not spawn processes, open
//! A2A channels, or unwrap TDF payloads. It owns the per-role adaptation
//! state and the per-role DecisionTrace, and exposes a way to record tool
//! outcomes against a specific role so each role's audit trail stays
//! isolated.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::skill_resolver;

use arkavo_arp::ArpDocument;
use arkavo_arp_runtime::{ArpRuntime, ToolOutcomeContext};
use arkavo_swarmkit::Manifest;
use uuid::Uuid;

use crate::derive::{DeriveOptions, derive_arp_for_role};

/// Per-role task envelope handed to a [`RoleTaskTransport`] for delivery.
#[derive(Debug, Clone)]
pub struct RoleTaskEnvelope {
    pub flight_id: Uuid,
    pub role_id: String,
    pub role_type: String,
    pub agent_did: String,
    pub task: String,
}

/// Pluggable transport for sending the first per-role task to its bound
/// agent. Implementations bridge to A2A in production and capture
/// envelopes in tests.
pub trait RoleTaskTransport: Send + Sync {
    fn dispatch<'a>(
        &'a self,
        envelope: RoleTaskEnvelope,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

/// Receipt for one dispatched first-task envelope. Returned by
/// [`SwarmFlight::dispatch_initial_tasks`] so callers can correlate
/// transport sends with manifest roles.
#[derive(Debug, Clone)]
pub struct DispatchHandle {
    pub role_id: String,
    pub agent_did: String,
}

/// Errors that can occur while *launching* a SwarmFlight. These reflect
/// validation against the manifest at flight construction time — once a
/// flight is launched, runtime operations against it use [`FlightError`]
/// instead.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("override role_id {0:?} does not appear in the manifest")]
    UnknownOverrideRole(String),
    #[error("manifest has no roles")]
    NoRoles,
    #[error("skill resolution failed for role {role:?}: {source}")]
    SkillResolution {
        role: String,
        #[source]
        source: skill_resolver::ResolveError,
    },
}

/// Errors that can occur while *operating* on a launched SwarmFlight —
/// e.g. recording a tool outcome against a role that does not exist.
/// Distinct from [`LaunchError`] so callers can distinguish a misconfigured
/// flight from a misuse of an already-launched one.
#[derive(Debug, thiserror::Error)]
pub enum FlightError {
    #[error("role_id {0:?} is not a role in this flight")]
    UnknownRole(String),
    #[error("dispatching first task for role {role_id:?} via transport: {message}")]
    DispatchFailed { role_id: String, message: String },
    #[error("role {role_id:?} has no bound agent — cannot dispatch")]
    NoBoundAgent { role_id: String },
}

/// Options applied at SwarmFlight launch time.
#[derive(Debug, Default)]
pub struct LaunchOptions {
    /// Per-role explicit ARP documents. Roles not listed get a default
    /// derived from `agent_provisioning` via `derive::derive_arp_for_role`.
    pub arp_overrides: HashMap<String, ArpDocument>,
    /// Tunables applied to derived ARP documents (no effect on overrides).
    pub derive_options: DeriveOptions,
    /// Optional explicit flight id. If `None`, a fresh UUID is generated.
    pub flight_id: Option<Uuid>,
    /// When `Some`, `SwarmFlight::launch` resolves and verifies every
    /// role's skills before constructing per-role ARP runtimes.
    ///
    /// `None` (the default) skips resolution; `RoleRuntime::resolved_skills()`
    /// returns an empty slice in that case.
    pub resolver_config: Option<skill_resolver::ResolverConfig>,
    /// role_id → bound agent DID. Set by the orchestrator after
    /// capability matching. Roles not listed have no bound agent and
    /// `dispatch_initial_tasks` will skip them. Surfaced via
    /// [`RoleRuntime::bound_agent_did`].
    pub role_agent_bindings: HashMap<String, String>,
}

/// Per-role runtime: the ARP runtime plus minimal manifest metadata so
/// callers can route by role_type without re-parsing the manifest. The
/// originating ARP document is retained so the AG-UI ARP panel and other
/// observers can render the same document the runtime was built from.
pub struct RoleRuntime {
    role_id: String,
    role_type: String,
    arp: Arc<ArpRuntime>,
    arp_document: ArpDocument,
    resolved_skills: Vec<skill_resolver::ResolvedSkill>,
    /// DID of the mesh agent the orchestrator assigned to this role.
    /// `None` for in-process / synthetic flights with no real agent
    /// behind each role.
    bound_agent_did: Option<String>,
    /// First task this role should perform per the manifest's
    /// coordination topology. Populated at launch from
    /// `objective.goal` + the role's role_type. Used by
    /// [`SwarmFlight::dispatch_initial_tasks`].
    initial_task: String,
}

impl RoleRuntime {
    pub fn role_id(&self) -> &str {
        &self.role_id
    }

    pub fn role_type(&self) -> &str {
        &self.role_type
    }

    pub fn arp(&self) -> &Arc<ArpRuntime> {
        &self.arp
    }

    /// The ARP document this role's runtime was built from — either an
    /// explicit override from `LaunchOptions::arp_overrides` or the default
    /// derived from `agent_provisioning`.
    pub fn arp_document(&self) -> &ArpDocument {
        &self.arp_document
    }

    /// Skills resolved at launch time. Empty when `LaunchOptions::resolver_config`
    /// was `None` (resolution skipped).
    pub fn resolved_skills(&self) -> &[skill_resolver::ResolvedSkill] {
        &self.resolved_skills
    }

    /// DID of the mesh agent assigned to this role, or `None` for an
    /// unbound (in-process / synthetic) flight.
    pub fn bound_agent_did(&self) -> Option<&str> {
        self.bound_agent_did.as_deref()
    }

    /// First task description for this role. Used by
    /// [`SwarmFlight::dispatch_initial_tasks`].
    pub fn initial_task(&self) -> &str {
        &self.initial_task
    }
}

/// A launched SwarmFlight. Holds one `ArpRuntime` per role, keyed by `role_id`.
pub struct SwarmFlight {
    flight_id: Uuid,
    kit_id: String,
    kit_name: String,
    roles: HashMap<String, RoleRuntime>,
    role_order: Vec<String>,
}

impl std::fmt::Debug for SwarmFlight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwarmFlight")
            .field("flight_id", &self.flight_id)
            .field("kit_id", &self.kit_id)
            .field("kit_name", &self.kit_name)
            .field("role_order", &self.role_order)
            .finish_non_exhaustive()
    }
}

impl SwarmFlight {
    /// Launch a flight from a parsed SwarmKit manifest.
    ///
    /// For each role in the manifest, build an `ArpRuntime` from either
    /// `options.arp_overrides[role_id]` or a default derived from the role's
    /// `agent_provisioning` block.
    pub fn launch(manifest: &Manifest, options: LaunchOptions) -> Result<Self, LaunchError> {
        if manifest.roles.is_empty() {
            return Err(LaunchError::NoRoles);
        }

        let role_ids: std::collections::HashSet<&str> =
            manifest.roles.iter().map(|r| r.id.as_str()).collect();
        for override_id in options.arp_overrides.keys() {
            if !role_ids.contains(override_id.as_str()) {
                return Err(LaunchError::UnknownOverrideRole(override_id.clone()));
            }
        }

        let role_count = manifest.roles.len();
        let mut roles = HashMap::with_capacity(role_count);
        let mut role_order = Vec::with_capacity(role_count);

        let resolved_per_role: HashMap<String, Vec<skill_resolver::ResolvedSkill>> =
            if let Some(cfg) = &options.resolver_config {
                let mut out = HashMap::with_capacity(manifest.roles.len());
                for role in &manifest.roles {
                    let mut resolved = Vec::with_capacity(role.skills.len());
                    for skill in &role.skills {
                        let r = skill_resolver::resolve_skill(skill, cfg).map_err(|e| {
                            LaunchError::SkillResolution {
                                role: role.id.clone(),
                                source: e,
                            }
                        })?;
                        resolved.push(r);
                    }
                    out.insert(role.id.clone(), resolved);
                }
                out
            } else {
                HashMap::new()
            };

        for role in &manifest.roles {
            let arp_doc = options
                .arp_overrides
                .get(&role.id)
                .cloned()
                .unwrap_or_else(|| {
                    derive_arp_for_role(
                        role,
                        &manifest.constraints.global_budget,
                        role_count,
                        options.derive_options,
                    )
                });
            let arp = Arc::new(ArpRuntime::from_document(&arp_doc));
            let initial_task = format!(
                "[{role_type}] {goal}",
                role_type = role.role_type,
                goal = manifest.objective.goal,
            );
            role_order.push(role.id.clone());
            roles.insert(
                role.id.clone(),
                RoleRuntime {
                    role_id: role.id.clone(),
                    role_type: role.role_type.clone(),
                    arp,
                    arp_document: arp_doc,
                    resolved_skills: resolved_per_role.get(&role.id).cloned().unwrap_or_default(),
                    bound_agent_did: options.role_agent_bindings.get(&role.id).cloned(),
                    initial_task,
                },
            );
        }

        Ok(Self {
            flight_id: options.flight_id.unwrap_or_else(Uuid::new_v4),
            kit_id: manifest.kit.id.clone(),
            kit_name: manifest.kit.name.clone(),
            roles,
            role_order,
        })
    }

    pub fn flight_id(&self) -> Uuid {
        self.flight_id
    }

    pub fn kit_id(&self) -> &str {
        &self.kit_id
    }

    pub fn kit_name(&self) -> &str {
        &self.kit_name
    }

    pub fn role(&self, role_id: &str) -> Option<&RoleRuntime> {
        self.roles.get(role_id)
    }

    /// Iterate roles in manifest order.
    pub fn roles(&self) -> impl Iterator<Item = &RoleRuntime> {
        self.role_order
            .iter()
            .filter_map(move |id| self.roles.get(id))
    }

    /// Record a tool outcome against the given role's ARP runtime. The flight
    /// id and role id are propagated into the DecisionTrace via context so
    /// the per-role audit trail carries flight provenance. When the role
    /// has a bound agent DID, that DID tags the trace entry; otherwise the
    /// role_id is used (preserving prior behavior for unbound flights).
    pub async fn record_tool_outcome(
        &self,
        role_id: &str,
        tool_name: &str,
        success: bool,
        quality: f64,
    ) -> Result<(), FlightError> {
        let role = self
            .roles
            .get(role_id)
            .ok_or_else(|| FlightError::UnknownRole(role_id.to_string()))?;
        let agent_for_trace = role.bound_agent_did.as_deref().unwrap_or(role_id);
        let ctx = ToolOutcomeContext::new()
            .with_task_id(self.flight_id)
            .with_agent_id(agent_for_trace);
        role.arp
            .record_tool_outcome_with(tool_name, success, quality, &ctx)
            .await;
        Ok(())
    }

    /// Dispatch the first task to every bound role via the supplied
    /// transport. Roles without a bound agent are silently skipped and
    /// excluded from the returned handles. A transport error for one
    /// role short-circuits the whole call so the orchestrator can react
    /// to partial-dispatch states cleanly.
    ///
    /// `transport` is bound as `?Sized` so callers can pass either a
    /// concrete type or `&dyn RoleTaskTransport` without an extra wrap.
    pub async fn dispatch_initial_tasks<T: RoleTaskTransport + ?Sized>(
        &self,
        transport: &T,
    ) -> Result<Vec<DispatchHandle>, FlightError> {
        let mut handles = Vec::new();
        for role_id in &self.role_order {
            let role = match self.roles.get(role_id) {
                Some(r) => r,
                None => continue,
            };
            let Some(did) = role.bound_agent_did.clone() else {
                continue;
            };
            let envelope = RoleTaskEnvelope {
                flight_id: self.flight_id,
                role_id: role.role_id.clone(),
                role_type: role.role_type.clone(),
                agent_did: did.clone(),
                task: role.initial_task.clone(),
            };
            transport
                .dispatch(envelope)
                .await
                .map_err(|message| FlightError::DispatchFailed {
                    role_id: role.role_id.clone(),
                    message,
                })?;
            handles.push(DispatchHandle {
                role_id: role.role_id.clone(),
                agent_did: did,
            });
        }
        Ok(handles)
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_swarmkit::parse_yaml;
    use arkavo_test_macros::spec;

    const KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "two-role"
  version: "0.1.0"
  authors: [{did: "did:web:example.com"}]
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "two roles"
  success_criteria: ["done"]
inputs: []
deliverables: [{name: "out", type: "json"}]
roles:
  - id: "alpha"
    role_type: "specialist"
    agent_provisioning: {}
  - id: "beta"
    role_type: "critic"
    agent_provisioning: {}
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

    #[spec("SK-010")]
    #[tokio::test]
    async fn launch_assigns_one_runtime_per_role() {
        let m = parse_yaml(KIT).unwrap();
        let f = SwarmFlight::launch(&m, LaunchOptions::default()).unwrap();
        assert_eq!(f.roles().count(), 2);
        assert!(f.role("alpha").is_some());
        assert!(f.role("beta").is_some());
    }

    #[spec("SK-011")]
    #[tokio::test]
    async fn outcomes_isolate_per_role() {
        let m = parse_yaml(KIT).unwrap();
        let f = SwarmFlight::launch(&m, LaunchOptions::default()).unwrap();

        f.record_tool_outcome("alpha", "tool_a", true, 0.95)
            .await
            .unwrap();
        f.record_tool_outcome("alpha", "tool_b", true, 0.92)
            .await
            .unwrap();
        f.record_tool_outcome("beta", "tool_c", false, 0.40)
            .await
            .unwrap();

        let alpha_trace = f.role("alpha").unwrap().arp().decision_trace();
        let beta_trace = f.role("beta").unwrap().arp().decision_trace();

        assert_eq!(alpha_trace.len(), 2);
        assert_eq!(beta_trace.len(), 1);

        let alpha_entries = alpha_trace.snapshot();
        for entry in &alpha_entries {
            assert_eq!(entry.agent_id, "alpha");
        }
        let beta_entries = beta_trace.snapshot();
        assert_eq!(beta_entries[0].agent_id, "beta");
        assert_eq!(beta_entries[0].outcome.success, Some(false));
    }

    #[spec("SK-015")]
    #[tokio::test]
    async fn flight_id_propagates_into_trace_task_id() {
        let m = parse_yaml(KIT).unwrap();
        let flight_id = Uuid::new_v4();
        let f = SwarmFlight::launch(
            &m,
            LaunchOptions {
                flight_id: Some(flight_id),
                ..LaunchOptions::default()
            },
        )
        .unwrap();
        f.record_tool_outcome("alpha", "tool_x", true, 0.9)
            .await
            .unwrap();
        let entries = f.role("alpha").unwrap().arp().decision_trace().snapshot();
        assert_eq!(entries[0].task_id, flight_id.to_string());
    }

    #[tokio::test]
    async fn unknown_role_id_in_record_returns_error() {
        let m = parse_yaml(KIT).unwrap();
        let f = SwarmFlight::launch(&m, LaunchOptions::default()).unwrap();
        let err = f
            .record_tool_outcome("ghost", "x", true, 0.9)
            .await
            .unwrap_err();
        // Runtime lookup failure on a launched flight surfaces as
        // FlightError::UnknownRole, distinct from LaunchError so callers
        // can tell construction-time misconfiguration apart from runtime
        // misuse.
        assert!(matches!(err, FlightError::UnknownRole(_)));
    }

    #[derive(Default)]
    struct CapturingTransport {
        sent: tokio::sync::Mutex<Vec<RoleTaskEnvelope>>,
        fail_role: Option<String>,
    }

    impl RoleTaskTransport for CapturingTransport {
        fn dispatch<'a>(
            &'a self,
            envelope: RoleTaskEnvelope,
        ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
            Box::pin(async move {
                if self
                    .fail_role
                    .as_deref()
                    .is_some_and(|r| r == envelope.role_id)
                {
                    return Err(format!("simulated failure for {}", envelope.role_id));
                }
                self.sent.lock().await.push(envelope);
                Ok(())
            })
        }
    }

    #[spec("SK-070")]
    #[tokio::test]
    async fn launch_with_role_agent_bindings_records_dids() {
        let m = parse_yaml(KIT).unwrap();
        let mut bindings = HashMap::new();
        bindings.insert("alpha".to_string(), "did:web:agent-a.arkavo.net".into());
        bindings.insert("beta".to_string(), "did:web:agent-b.arkavo.net".into());
        let f = SwarmFlight::launch(
            &m,
            LaunchOptions {
                role_agent_bindings: bindings,
                ..LaunchOptions::default()
            },
        )
        .unwrap();
        assert_eq!(
            f.role("alpha").unwrap().bound_agent_did(),
            Some("did:web:agent-a.arkavo.net")
        );
        assert_eq!(
            f.role("beta").unwrap().bound_agent_did(),
            Some("did:web:agent-b.arkavo.net")
        );
    }

    #[spec("SK-073")]
    #[tokio::test]
    async fn dispatch_initial_tasks_calls_transport_per_bound_role() {
        let m = parse_yaml(KIT).unwrap();
        let mut bindings = HashMap::new();
        bindings.insert("alpha".to_string(), "did:web:agent-a.arkavo.net".into());
        bindings.insert("beta".to_string(), "did:web:agent-b.arkavo.net".into());
        let f = SwarmFlight::launch(
            &m,
            LaunchOptions {
                role_agent_bindings: bindings,
                ..LaunchOptions::default()
            },
        )
        .unwrap();
        let transport = CapturingTransport::default();
        let handles = f.dispatch_initial_tasks(&transport).await.unwrap();
        assert_eq!(handles.len(), 2);
        let sent = transport.sent.lock().await;
        assert_eq!(sent.len(), 2);
        // Manifest order preserved.
        assert_eq!(sent[0].role_id, "alpha");
        assert_eq!(sent[0].agent_did, "did:web:agent-a.arkavo.net");
        assert!(sent[0].task.contains("two roles"));
        assert_eq!(sent[1].role_id, "beta");
        assert_eq!(sent[1].agent_did, "did:web:agent-b.arkavo.net");
    }

    #[tokio::test]
    async fn dispatch_initial_tasks_skips_unbound_roles() {
        let m = parse_yaml(KIT).unwrap();
        let mut bindings = HashMap::new();
        bindings.insert("alpha".to_string(), "did:web:agent-a.arkavo.net".into());
        let f = SwarmFlight::launch(
            &m,
            LaunchOptions {
                role_agent_bindings: bindings,
                ..LaunchOptions::default()
            },
        )
        .unwrap();
        let transport = CapturingTransport::default();
        let handles = f.dispatch_initial_tasks(&transport).await.unwrap();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].role_id, "alpha");
        let sent = transport.sent.lock().await;
        assert_eq!(sent.len(), 1);
    }

    #[tokio::test]
    async fn dispatch_initial_tasks_propagates_transport_failure() {
        let m = parse_yaml(KIT).unwrap();
        let mut bindings = HashMap::new();
        bindings.insert("alpha".to_string(), "did:web:agent-a.arkavo.net".into());
        bindings.insert("beta".to_string(), "did:web:agent-b.arkavo.net".into());
        let f = SwarmFlight::launch(
            &m,
            LaunchOptions {
                role_agent_bindings: bindings,
                ..LaunchOptions::default()
            },
        )
        .unwrap();
        let transport = CapturingTransport {
            fail_role: Some("alpha".to_string()),
            ..Default::default()
        };
        let err = f.dispatch_initial_tasks(&transport).await.unwrap_err();
        match err {
            FlightError::DispatchFailed { role_id, .. } => assert_eq!(role_id, "alpha"),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn record_tool_outcome_uses_bound_did_when_present() {
        let m = parse_yaml(KIT).unwrap();
        let mut bindings = HashMap::new();
        bindings.insert("alpha".to_string(), "did:web:agent-a.arkavo.net".into());
        let f = SwarmFlight::launch(
            &m,
            LaunchOptions {
                role_agent_bindings: bindings,
                ..LaunchOptions::default()
            },
        )
        .unwrap();
        f.record_tool_outcome("alpha", "tool_x", true, 0.9)
            .await
            .unwrap();
        let entries = f.role("alpha").unwrap().arp().decision_trace().snapshot();
        assert_eq!(entries[0].agent_id, "did:web:agent-a.arkavo.net");
        // Unbound role still uses role_id for backward compatibility.
        f.record_tool_outcome("beta", "tool_y", true, 0.9)
            .await
            .unwrap();
        let beta_entries = f.role("beta").unwrap().arp().decision_trace().snapshot();
        assert_eq!(beta_entries[0].agent_id, "beta");
    }

    #[spec("SK-014")]
    #[tokio::test]
    async fn unknown_override_id_rejected_at_launch() {
        let m = parse_yaml(KIT).unwrap();
        let mut overrides = HashMap::new();
        overrides.insert(
            "ghost".to_string(),
            crate::derive::derive_arp_for_role(
                &m.roles[0],
                &m.constraints.global_budget,
                m.roles.len(),
                DeriveOptions::default(),
            ),
        );
        let err = SwarmFlight::launch(
            &m,
            LaunchOptions {
                arp_overrides: overrides,
                ..LaunchOptions::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, LaunchError::UnknownOverrideRole(_)));
    }
}
