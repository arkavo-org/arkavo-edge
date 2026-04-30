//! SwarmFlight — a launched SwarmKit with one ArpRuntime per role.
//!
//! The runtime is a logical orchestrator. It does not spawn processes, open
//! A2A channels, or unwrap TDF payloads. It owns the per-role adaptation
//! state and the per-role DecisionTrace, and exposes a way to record tool
//! outcomes against a specific role so each role's audit trail stays
//! isolated.

use std::collections::HashMap;
use std::sync::Arc;

use arkavo_arp::ArpDocument;
use arkavo_arp_runtime::{ArpRuntime, ToolOutcomeContext};
use arkavo_swarmkit::Manifest;
use uuid::Uuid;

use crate::derive::{DeriveOptions, derive_arp_for_role};

/// Errors that can occur while launching a SwarmFlight.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("override role_id {0:?} does not appear in the manifest")]
    UnknownOverrideRole(String),
    #[error("manifest has no roles")]
    NoRoles,
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
            role_order.push(role.id.clone());
            roles.insert(
                role.id.clone(),
                RoleRuntime {
                    role_id: role.id.clone(),
                    role_type: role.role_type.clone(),
                    arp,
                    arp_document: arp_doc,
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
    /// the per-role audit trail carries flight provenance.
    pub async fn record_tool_outcome(
        &self,
        role_id: &str,
        tool_name: &str,
        success: bool,
        quality: f64,
    ) -> Result<(), LaunchError> {
        let role = self
            .roles
            .get(role_id)
            .ok_or_else(|| LaunchError::UnknownOverrideRole(role_id.to_string()))?;
        let ctx = ToolOutcomeContext::new()
            .with_task_id(self.flight_id)
            .with_agent_id(role_id);
        role.arp
            .record_tool_outcome_with(tool_name, success, quality, &ctx)
            .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_swarmkit::parse_yaml;

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

    #[tokio::test]
    async fn launch_assigns_one_runtime_per_role() {
        let m = parse_yaml(KIT).unwrap();
        let f = SwarmFlight::launch(&m, LaunchOptions::default()).unwrap();
        assert_eq!(f.roles().count(), 2);
        assert!(f.role("alpha").is_some());
        assert!(f.role("beta").is_some());
    }

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
        assert!(matches!(err, LaunchError::UnknownOverrideRole(_)));
    }

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
