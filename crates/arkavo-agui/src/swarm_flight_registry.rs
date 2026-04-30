//! Active SwarmFlight registry for the AG-UI gateway.
//!
//! Holds every flight currently running in-process and keeps the
//! `ArpHandler` in sync. Registering a flight makes each of its roles
//! addressable in the panel's per-agent dropdown (under
//! `flight:<flight_id>:<role_id>`); deregistering drops them.
//!
//! The flight itself remains owned by whoever launched it — this registry
//! holds an `Arc<SwarmFlight>` so the panel observes the same per-role
//! `ArpRuntime` instances the orchestrator is mutating.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use arkavo_swarmkit::{Manifest, parse_json, parse_yaml};
use arkavo_swarmkit_runtime::{LaunchOptions, SwarmFlight};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::arp_handler::{ArpHandler, FlightRoleRegistration};

/// Holds every SwarmFlight currently active in this gateway process.
#[derive(Default)]
pub struct SwarmFlightRegistry {
    flights: RwLock<HashMap<Uuid, Arc<SwarmFlight>>>,
}

impl SwarmFlightRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a launched flight. For each role, attach the role's
    /// runtime + ARP document to the handler so the panel can render it.
    pub async fn register(&self, flight: Arc<SwarmFlight>, arp_handler: &ArpHandler) {
        let flight_id = flight.flight_id();
        let kit_id = flight.kit_id().to_string();
        let kit_name = flight.kit_name().to_string();

        for role in flight.roles() {
            arp_handler
                .attach_flight_role(FlightRoleRegistration {
                    flight_id,
                    kit_id: kit_id.clone(),
                    kit_name: kit_name.clone(),
                    role_id: role.role_id().to_string(),
                    role_type: role.role_type().to_string(),
                    arp_doc: role.arp_document().clone(),
                    runtime: role.arp().clone(),
                })
                .await;
        }

        self.flights.write().await.insert(flight_id, flight);
    }

    /// Drop a flight from the registry and remove all of its roles from
    /// the handler. Call this when a SwarmFlight terminates.
    pub async fn deregister(&self, flight_id: Uuid, arp_handler: &ArpHandler) {
        self.flights.write().await.remove(&flight_id);
        arp_handler.remove_flight(flight_id).await;
    }

    pub async fn flight_count(&self) -> usize {
        self.flights.read().await.len()
    }

    pub async fn get(&self, flight_id: Uuid) -> Option<Arc<SwarmFlight>> {
        self.flights.read().await.get(&flight_id).cloned()
    }
}

/// Errors that can happen during environment-driven auto-launch.
#[derive(Debug, thiserror::Error)]
pub enum AutoLaunchError {
    #[error("read manifest at {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("parse manifest at {path}: {message}")]
    Parse { path: String, message: String },
    #[error("launch flight from {path}: {message}")]
    Launch { path: String, message: String },
}

/// Read `ARKAVO_SWARMKIT_PATH`, parse the manifest, launch a flight, and
/// register it with the gateway. Returns `Ok(None)` when the env var is
/// unset (the common case); errors are non-fatal at the call site.
pub async fn auto_launch_from_environment(
    registry: &SwarmFlightRegistry,
    arp_handler: &ArpHandler,
) -> Result<Option<Uuid>, AutoLaunchError> {
    let Some(path) = std::env::var("ARKAVO_SWARMKIT_PATH").ok() else {
        return Ok(None);
    };
    let flight = launch_from_path(Path::new(&path))?;
    let flight_id = flight.flight_id();
    registry.register(Arc::new(flight), arp_handler).await;
    Ok(Some(flight_id))
}

/// Parse a manifest from disk and launch a flight against it. Public for
/// callers that want to drive a flight without going through the env var.
pub fn launch_from_path(path: &Path) -> Result<SwarmFlight, AutoLaunchError> {
    let raw = std::fs::read_to_string(path).map_err(|e| AutoLaunchError::Read {
        path: path.display().to_string(),
        source: e,
    })?;
    let manifest = parse_manifest(path, &raw)?;
    SwarmFlight::launch(&manifest, LaunchOptions::default()).map_err(|e| AutoLaunchError::Launch {
        path: path.display().to_string(),
        message: e.to_string(),
    })
}

fn parse_manifest(path: &Path, raw: &str) -> Result<Manifest, AutoLaunchError> {
    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let parsed = if is_json {
        parse_json(raw)
    } else {
        parse_yaml(raw)
    };
    parsed.map_err(|e| AutoLaunchError::Parse {
        path: path.display().to_string(),
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "registry-test-kit"
  version: "0.1.0"
  authors: [{did: "did:web:example.com"}]
  created: "2026-04-30T00:00:00Z"
  expires: "2026-05-30T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "exercise the flight registry"
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
    async fn register_attaches_all_roles_to_handler() {
        let registry = SwarmFlightRegistry::new();
        let handler = ArpHandler::new();
        let manifest = parse_yaml(KIT).unwrap();
        let flight = Arc::new(SwarmFlight::launch(&manifest, LaunchOptions::default()).unwrap());
        let flight_id = flight.flight_id();

        registry.register(flight, &handler).await;

        assert_eq!(registry.flight_count().await, 1);
        let snap = handler.snapshot().await;
        assert_eq!(snap.agents.len(), 2);
        assert!(
            snap.agents
                .iter()
                .any(|a| a.flight_context.as_ref().unwrap().role_id == "alpha")
        );
        assert!(
            snap.agents
                .iter()
                .any(|a| a.flight_context.as_ref().unwrap().role_id == "beta")
        );
        assert!(
            snap.agents
                .iter()
                .all(|a| a.flight_context.as_ref().unwrap().flight_id == flight_id.to_string())
        );
    }

    #[tokio::test]
    async fn deregister_removes_roles_and_registry_entry() {
        let registry = SwarmFlightRegistry::new();
        let handler = ArpHandler::new();
        let manifest = parse_yaml(KIT).unwrap();
        let flight = Arc::new(SwarmFlight::launch(&manifest, LaunchOptions::default()).unwrap());
        let flight_id = flight.flight_id();

        registry.register(flight, &handler).await;
        registry.deregister(flight_id, &handler).await;

        assert_eq!(registry.flight_count().await, 0);
        assert!(handler.snapshot().await.agents.is_empty());
    }

    #[tokio::test]
    async fn auto_launch_unset_env_var_returns_none() {
        // Save and clear so this test is not order-dependent.
        let prev = std::env::var("ARKAVO_SWARMKIT_PATH").ok();
        // SAFETY: tests are single-threaded under tokio::test current_thread
        // and we restore after.
        unsafe {
            std::env::remove_var("ARKAVO_SWARMKIT_PATH");
        }

        let registry = SwarmFlightRegistry::new();
        let handler = ArpHandler::new();
        let result = auto_launch_from_environment(&registry, &handler).await;

        if let Some(p) = prev {
            unsafe {
                std::env::set_var("ARKAVO_SWARMKIT_PATH", p);
            }
        }

        assert!(matches!(result, Ok(None)));
        assert_eq!(registry.flight_count().await, 0);
    }

    #[tokio::test]
    async fn launch_from_path_loads_yaml_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.swarmkit.yaml");
        std::fs::write(&path, KIT).unwrap();

        let flight = launch_from_path(&path).expect("launch");
        assert_eq!(flight.kit_name(), "registry-test-kit");
        assert_eq!(flight.roles().count(), 2);
    }

    #[tokio::test]
    async fn launch_from_path_reports_invalid_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.swarmkit.yaml");
        std::fs::write(&path, "not: [a, valid, manifest").unwrap();

        let err = launch_from_path(&path).unwrap_err();
        assert!(matches!(err, AutoLaunchError::Parse { .. }));
    }
}
