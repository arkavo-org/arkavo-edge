//! End-to-end integration test for the `github-ops-kit` apply flow.
//!
//! Drives [`apply_kit`] against the *real* `examples/github-ops-kit`
//! manifest with in-process stubs for the encryptor, shipper, and task
//! transport, and an `AgentRegistry` pre-populated with one fake agent
//! per role. No live network, no Iroh node, no GitHub API, no KAS.
//!
//! Asserts the orchestrator-layer integration the WS-E sender scope owns:
//! all four roles bind to agents, four first-task envelopes dispatch, and
//! the `repo_maintainer` envelope is scope-pinned to the triggering
//! org/repo while non-maintainer envelopes are not.

// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace's
// disallowed-methods lint flags; allowed here as in the other e2e tests.
#![allow(clippy::disallowed_methods)]

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use arkavo_orchestrator::{
    BundleEncryptor, BundleShipper, EnvTokenVault, RepoScope, RoleCapabilityMatcher, apply_kit,
};
use arkavo_protocol::AgentSpecializationBundle;
use arkavo_protocol::agent_registry::AgentRegistry;
use arkavo_swarmkit_runtime::{RoleTaskEnvelope, RoleTaskTransport};
use async_trait::async_trait;

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/github-ops-kit/github-ops-kit.swarmkit.yaml")
}

// ── Stubs ────────────────────────────────────────────────────────────────────

/// Encryptor that returns the bundle's canonical JSON unencrypted — enough
/// to exercise the build+ship pipeline without a KAS.
struct IdentityEncryptor;

#[async_trait]
impl BundleEncryptor for IdentityEncryptor {
    async fn wrap(
        &self,
        bundle: &AgentSpecializationBundle,
        _recipient_did: &str,
    ) -> Result<Vec<u8>, String> {
        bundle.to_canonical_json().map_err(|e| e.to_string())
    }
}

/// Shipper that drops the bytes on the floor — the test asserts on
/// dispatched task envelopes, not on delivery.
struct NullShipper;

#[async_trait]
impl BundleShipper for NullShipper {
    async fn ship(&self, _agent_did: &str, _tdf_bytes: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

/// Transport that records every dispatched envelope for assertions.
struct CapturingTransport {
    envelopes: Arc<Mutex<Vec<RoleTaskEnvelope>>>,
}

impl CapturingTransport {
    fn new() -> (Self, Arc<Mutex<Vec<RoleTaskEnvelope>>>) {
        let store = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                envelopes: Arc::clone(&store),
            },
            store,
        )
    }
}

impl RoleTaskTransport for CapturingTransport {
    fn dispatch<'a>(
        &'a self,
        envelope: RoleTaskEnvelope,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        self.envelopes.lock().unwrap().push(envelope);
        Box::pin(async { Ok(()) })
    }
}

// ── Registry: one agent per github-ops-kit role capability ───────────────────

/// Register four agents whose capabilities match the manifest's role skill
/// ids (what `RoleCapabilityMatcher::required_capabilities` resolves first).
async fn populated_registry() -> Arc<AgentRegistry> {
    let registry = Arc::new(AgentRegistry::new());
    let agents = [
        ("did:web:dispatcher", "skill:triage-github-event"),
        ("did:web:reviewer", "skill:pr-review"),
        ("did:web:runner", "skill:run-pr-tests"),
        ("did:web:maintainer", "skill:repo-housekeeping"),
    ];
    for (did, skill) in agents {
        registry
            .register_agent(
                did.to_string(),
                did.to_string(),
                "test agent".to_string(),
                vec![
                    skill.to_string(),
                    "arkavo-github".to_string(),
                    "arkavo-git".to_string(),
                ],
                None,
                std::collections::HashMap::new(),
                Some("http://127.0.0.1:9000".to_string()),
            )
            .await
            .expect("register agent");
    }
    registry
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn github_ops_kit_apply_binds_all_four_roles() {
    let registry = populated_registry().await;
    let matcher = RoleCapabilityMatcher::new(registry);
    let vault = EnvTokenVault;
    let encryptor = IdentityEncryptor;
    let shipper = NullShipper;
    let (transport, _captured) = CapturingTransport::new();
    let scope = RepoScope::new("test-org", "test-repo");

    let applied = apply_kit(
        &manifest_path(),
        &matcher,
        &vault,
        &encryptor,
        &shipper,
        &transport,
        Some(&scope),
    )
    .await
    .expect("apply_kit must succeed with a fully populated registry");

    assert_eq!(
        applied.bindings.len(),
        4,
        "all four github-ops-kit roles must bind: {:?}",
        applied
            .bindings
            .iter()
            .map(|b| &b.role_id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        applied.dispatch_handles.len(),
        4,
        "one first-task dispatch per bound role"
    );
}

#[tokio::test]
async fn repo_maintainer_envelope_is_scoped() {
    let registry = populated_registry().await;
    let matcher = RoleCapabilityMatcher::new(registry);
    let vault = EnvTokenVault;
    let encryptor = IdentityEncryptor;
    let shipper = NullShipper;
    let (transport, captured) = CapturingTransport::new();
    let scope = RepoScope::new("test-org", "test-repo");

    apply_kit(
        &manifest_path(),
        &matcher,
        &vault,
        &encryptor,
        &shipper,
        &transport,
        Some(&scope),
    )
    .await
    .expect("apply_kit must succeed");

    let envelopes = captured.lock().unwrap();

    let maintainer = envelopes
        .iter()
        .find(|e| e.role_type == "maintainer")
        .expect("a maintainer-role envelope must be dispatched");
    assert!(
        maintainer
            .task
            .starts_with("[SCOPE: owner=test-org repo=test-repo"),
        "maintainer task must begin with the scope preamble, got: {:?}",
        maintainer.task
    );

    for env in envelopes.iter().filter(|e| e.role_type != "maintainer") {
        assert!(
            !env.task.starts_with("[SCOPE:"),
            "non-maintainer role {:?} must not carry a scope preamble",
            env.role_id
        );
    }
}
