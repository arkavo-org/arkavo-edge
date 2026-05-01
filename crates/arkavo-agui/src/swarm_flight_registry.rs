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
    #[error(
        "{path} is a .tdf-encoded SwarmKit but arkavo-agui was built without the `tdf` feature; \
         rebuild with --features tdf to auto-launch encrypted kits"
    )]
    TdfFeatureDisabled { path: String },
    #[error("decrypt .tdf manifest at {path}: {message}")]
    TdfUnwrap { path: String, message: String },
}

/// Read `ARKAVO_SWARMKIT_PATH`, parse the manifest, launch a flight, and
/// register it with the gateway. Returns `Ok(None)` when the env var is
/// unset (the common case); errors are non-fatal at the call site.
///
/// Path dispatch:
/// * `*.swarmkit.yaml`, `*.yaml`, `*.yml` → plain YAML
/// * `*.json` → plain JSON
/// * `*.swarmkit.tdf`, `*.tdf` → encrypted TDF envelope (requires the
///   `tdf` feature; surfaces `TdfFeatureDisabled` otherwise)
pub async fn auto_launch_from_environment(
    registry: &SwarmFlightRegistry,
    arp_handler: &ArpHandler,
) -> Result<Option<Uuid>, AutoLaunchError> {
    let Some(path) = std::env::var("ARKAVO_SWARMKIT_PATH").ok() else {
        return Ok(None);
    };
    let path_buf = Path::new(&path).to_path_buf();
    let flight = if is_tdf_path(&path_buf) {
        launch_from_tdf_path(&path_buf).await?
    } else {
        launch_from_path(&path_buf)?
    };
    let flight_id = flight.flight_id();
    registry.register(Arc::new(flight), arp_handler).await;
    Ok(Some(flight_id))
}

/// Parse a plaintext manifest from disk and launch a flight against it.
/// For `.tdf`-encoded kits use [`launch_from_tdf_path`].
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

/// Decrypt a `.swarmkit.tdf` envelope with the default `OpenTdfService`
/// and launch a flight against the recovered manifest.
///
/// Available only with the `tdf` feature. For tighter control over the
/// decryptor (custom KAS URL) or to add a KAS health gate before
/// decryption, use the lower-level helpers in
/// `arkavo-swarmkit-runtime`'s `tdf` module directly.
#[cfg(feature = "tdf")]
pub async fn launch_from_tdf_path(path: &Path) -> Result<SwarmFlight, AutoLaunchError> {
    use arkavo_swarmkit_runtime::{OpenTdfService, unwrap_manifest_from_path};

    let decryptor = OpenTdfService::default();
    let manifest = unwrap_manifest_from_path(&decryptor, path)
        .await
        .map_err(|e| AutoLaunchError::TdfUnwrap {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
    SwarmFlight::launch(&manifest, LaunchOptions::default()).map_err(|e| AutoLaunchError::Launch {
        path: path.display().to_string(),
        message: e.to_string(),
    })
}

/// Stub returned when the gateway is built without the `tdf` feature
/// but a `.tdf` path is supplied. Surfaces a clear error so operators
/// know to rebuild rather than guessing why the flight didn't start.
#[cfg(not(feature = "tdf"))]
pub async fn launch_from_tdf_path(path: &Path) -> Result<SwarmFlight, AutoLaunchError> {
    Err(AutoLaunchError::TdfFeatureDisabled {
        path: path.display().to_string(),
    })
}

/// `true` when the path's extension marks it as a TDF envelope. Matches
/// either bare `.tdf` or the recommended `.swarmkit.tdf` double extension.
pub fn is_tdf_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".tdf") || lower.ends_with(".swarmkit.tdf")
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
    use arkavo_test_macros::spec;

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

    #[spec("SK-020")]
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

    #[spec("SK-021")]
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

    #[spec("SK-022")]
    #[serial_test::serial(env_arkavo_swarmkit_path)]
    #[tokio::test]
    async fn auto_launch_unset_env_var_returns_none() {
        // serial_test ensures no other test mutates ARKAVO_SWARMKIT_PATH
        // concurrently. The default cargo test harness runs test functions
        // across multiple threads even when each one uses
        // `tokio::test(flavor = "current_thread")`; the flavor only governs
        // the tokio runtime *inside* one test, not the harness scheduler.
        let prev = std::env::var("ARKAVO_SWARMKIT_PATH").ok();
        // SAFETY: serial_test serializes any test in this binary that
        // touches the same key, so set_var/remove_var calls do not race.
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

    #[spec("SK-022")]
    #[tokio::test]
    async fn launch_from_path_loads_yaml_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.swarmkit.yaml");
        std::fs::write(&path, KIT).unwrap();

        let flight = launch_from_path(&path).expect("launch");
        assert_eq!(flight.kit_name(), "registry-test-kit");
        assert_eq!(flight.roles().count(), 2);
    }

    #[spec("SK-061")]
    #[test]
    fn is_tdf_path_recognises_extensions() {
        use std::path::PathBuf;
        assert!(is_tdf_path(&PathBuf::from("kit.tdf")));
        assert!(is_tdf_path(&PathBuf::from("kit.swarmkit.tdf")));
        assert!(is_tdf_path(&PathBuf::from("/abs/path/Kit.SWARMKIT.TDF")));
        assert!(!is_tdf_path(&PathBuf::from("kit.swarmkit.yaml")));
        assert!(!is_tdf_path(&PathBuf::from("kit.json")));
        assert!(!is_tdf_path(&PathBuf::from("kit")));
    }

    #[cfg(not(feature = "tdf"))]
    #[spec("SK-062")]
    #[tokio::test]
    async fn launch_from_tdf_path_errors_when_feature_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kit.swarmkit.tdf");
        std::fs::write(&path, b"unused").unwrap();
        let err = launch_from_tdf_path(&path).await.unwrap_err();
        assert!(matches!(err, AutoLaunchError::TdfFeatureDisabled { .. }));
    }

    #[cfg(feature = "tdf")]
    #[spec("SK-062")]
    #[tokio::test]
    async fn launch_from_tdf_path_round_trips_through_wrap_to_path() {
        use arkavo_swarmkit_runtime::{
            OpenTdfService, swarmkit_orchestrator_policy, wrap_manifest_to_path,
        };
        // We can't decrypt with OpenTdfService here without a working KAS,
        // so this test verifies the *dispatch* path: a .swarmkit.tdf file
        // produced by our own helpers is recognised, the unwrap is
        // attempted, and any failure surfaces as TdfUnwrap rather than
        // a generic Read or Parse error.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kit.swarmkit.tdf");
        let manifest = parse_yaml(KIT).unwrap();
        let svc = OpenTdfService::default();
        let policy = swarmkit_orchestrator_policy(None, None).unwrap();
        // wrap_manifest_to_path requires a working OpenTdfService; if it
        // can't reach the default KAS the wrap itself fails — in that
        // case skip rather than masking real test failures.
        let Ok(()) = wrap_manifest_to_path(&manifest, &svc, &policy, &path).await else {
            eprintln!("wrap_manifest_to_path failed (no KAS); skipping dispatch test");
            return;
        };
        // Now dispatch path: file exists with .tdf extension, should be
        // recognised. The unwrap will likely fail (no KAS in test env),
        // but the error must be TdfUnwrap, not a generic Read/Parse.
        match launch_from_tdf_path(&path).await {
            Ok(_) => {}
            Err(AutoLaunchError::TdfUnwrap { .. }) => {}
            Err(other) => panic!("expected Ok or TdfUnwrap, got {other:?}"),
        }
    }

    #[spec("SK-022")]
    #[tokio::test]
    async fn launch_from_path_reports_invalid_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.swarmkit.yaml");
        std::fs::write(&path, "not: [a, valid, manifest").unwrap();

        let err = launch_from_path(&path).unwrap_err();
        assert!(matches!(err, AutoLaunchError::Parse { .. }));
    }
}
