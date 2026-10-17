// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use super::*;
use arkavo_crypto::AgentKeypair;
use arkavo_swarmkit_delegation::{BindingMode, Revocation, SignedDelegation};
use arkavo_test_macros::spec;
use chrono::TimeDelta;
use uuid::Uuid;

const KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: "computed in manifest()"
  name: image-swarm
  version: 0.1.0
  authors: [{did: "did:web:local.arkavo.dev"}]
  created: "2026-10-16T00:00:00Z"
  expires: "2027-01-14T00:00:00Z"
  nonce: "ZgOj5i5lHm-ZxfrCwEr5rQ"
objective:
  goal: Make game art on request
roles:
  - id: artist
    role_type: operator
    description: Generates game art on this Mac
    agent_provisioning:
      model: {family: gemma, size: E2B}
    skills:
      - id: skill:identity
        version: 0.1.0
        source: inline
        payload: {name: identity, description: System identity, instructions: Make the images people ask for.}
    mcp_tools:
      - {server: arkavo, auth: none, tools: [generate_image]}
    host_requirements: [image_generation]
  - id: writer
    role_type: operator
    description: Asks the artist for images
    agent_provisioning:
      model: {family: gemma, size: E2B}
    mcp_tools:
      - {server: arkavo, auth: none, tools: [generate_image]}
  - id: unknown
    role_type: operator
    agent_provisioning:
      model: {family: nonesuch, size: 1B}
coordination: {topology: hub-spoke, protocol: a2a-jsonrpc-2.0, routing: {strategy: static}}
constraints:
  global_budget: {max_wallclock_seconds: 1800, max_total_tokens: 100000, max_cost_usd: 0.0}
  network: {egress_allowed: false}
completion: {rules: [objective addressed], on_failure: abort, max_retries: 0}
provenance: {signatures: []}
"#;

fn manifest() -> arkavo_swarmkit::Manifest {
    let mut manifest: arkavo_swarmkit::Manifest = serde_yaml::from_str(KIT).unwrap();
    manifest.kit.id = arkavo_swarmkit::kit_id_for(&manifest).unwrap();
    manifest
}

fn envelope(orchestrator: &AgentKeypair, specialist: &str, role_id: &str) -> SignedDelegation {
    let manifest = manifest();
    let role = manifest.roles.iter().find(|r| r.id == role_id).unwrap();
    Delegation::for_role(
        &manifest,
        role,
        orchestrator.public_key().to_did_key(),
        specialist.to_string(),
        Uuid::new_v4(),
        Utc::now(),
        TimeDelta::hours(1),
    )
    .unwrap()
    .sign(orchestrator)
    .unwrap()
}

async fn transport() -> arkavo_tdf_iroh::IrohTransport {
    let node = Arc::new(arkavo_tdf_iroh::IrohNode::new(
        arkavo_tdf_iroh::IrohNodeConfig::default(),
    ));
    node.start().await.expect("node starts");
    arkavo_tdf_iroh::IrohTransport::new(node)
}

/// Gemma 4 E2B is the one model this test host runs.
fn resolver() -> ModelResolver {
    Arc::new(|family, size| {
        (family == "gemma" && size == Some("E2B")).then(|| "gemma-4-e2b".to_string())
    })
}

struct Agent {
    _dir: tempfile::TempDir,
    dir: PathBuf,
    key: String,
    metadata: Arc<RwLock<AgentMetadata>>,
}

fn agent() -> Agent {
    let dir = tempfile::tempdir().unwrap();
    let metadata = AgentMetadata {
        name: "host-folder".into(),
        purpose: "A general-purpose AI agent".into(),
        model: "gemma-4-12b".into(),
        ..AgentMetadata::default()
    };
    Agent {
        dir: dir.path().to_path_buf(),
        _dir: dir,
        key: AgentKeypair::generate().public_key().to_did_key(),
        metadata: Arc::new(RwLock::new(metadata)),
    }
}

async fn open(
    agent: &Agent,
    enlistable: bool,
    assets: Option<Arc<AssetService>>,
) -> Arc<SpecialistService> {
    let config = SpecialistConfig {
        key: agent.key.clone(),
        state_dir: agent.dir.clone(),
        enlistable,
        operator_orchestrator: None,
        resolve_model: resolver(),
    };
    SpecialistService::open(config, agent.metadata.clone(), assets)
        .await
        .unwrap()
}

fn reason(error: &ErrorObjectOwned) -> RefusalReason {
    assert_eq!(error.code(), REFUSAL_CODE, "{error:?}");
    let data: Refusal = serde_json::from_str(error.data().unwrap().get()).unwrap();
    data.reason
}

#[spec("SKD-010", "SKD-011")]
#[tokio::test]
async fn a_role_sets_purpose_model_and_tools_and_ends_with_its_revocation() {
    let a = agent();
    let service = open(&a, true, None).await;
    let status = service.status().await;
    assert_eq!(status.specialist, a.key);
    assert_eq!(status.binds_by, [BindingMode::FirstUse]);
    assert!(
        status.capabilities.is_empty(),
        "no data plane, no generation"
    );

    let orchestrator = AgentKeypair::generate();
    let signed = envelope(&orchestrator, &a.key, "writer");
    let ready = service.delegate(signed.wire().clone()).await.unwrap();
    assert_eq!(ready.role_id, "writer");
    assert_eq!(ready.envelope_blake3, signed.envelope_blake3());
    {
        let meta = a.metadata.read().await;
        assert_eq!(
            meta.purpose,
            "Make game art on request\n\nAsks the artist for images"
        );
        assert_eq!(meta.model, "gemma-4-e2b");
        assert_eq!(
            meta.description.as_deref(),
            Some("Asks the artist for images")
        );
        assert_eq!(meta.granted_tools, ["generate_image"]);
        assert!(meta.specialized);
    }
    let status = service.status().await;
    assert_eq!(status.delegation.unwrap().role_id, "writer");
    assert_eq!(
        status.binding.unwrap().orchestrator,
        orchestrator.public_key().to_did_key()
    );

    let revocation = Revocation::new(
        orchestrator.public_key().to_did_key(),
        a.key.clone(),
        Some(ready.flight_id),
        false,
        Utc::now(),
    )
    .sign(&orchestrator)
    .unwrap();
    service.revoke(revocation.wire().clone()).await.unwrap();
    let meta = a.metadata.read().await;
    assert_eq!(meta.purpose, "A general-purpose AI agent");
    assert_eq!(meta.model, "gemma-4-12b");
    assert!(meta.description.is_none());
    assert!(meta.granted_tools.is_empty() && !meta.specialized);
}

#[spec("SKD-011")]
#[tokio::test]
async fn a_host_refuses_a_role_it_cannot_take_and_stays_unbound() {
    let a = agent();
    let service = open(&a, true, None).await;
    let orchestrator = AgentKeypair::generate();

    let error = service
        .delegate(envelope(&orchestrator, &a.key, "artist").wire().clone())
        .await
        .unwrap_err();
    assert_eq!(reason(&error), RefusalReason::HostUnsuitable);
    let error = service
        .delegate(envelope(&orchestrator, &a.key, "unknown").wire().clone())
        .await
        .unwrap_err();
    assert_eq!(reason(&error), RefusalReason::HostUnsuitable);
    assert!(
        service.status().await.binding.is_none(),
        "a refused role binds nothing"
    );
    assert_eq!(
        a.metadata.read().await.purpose,
        "A general-purpose AI agent"
    );

    let error = service
        .delegate(
            envelope(&orchestrator, "did:key:z6Mkother", "writer")
                .wire()
                .clone(),
        )
        .await
        .unwrap_err();
    assert_eq!(reason(&error), RefusalReason::WrongSpecialist);
}

#[spec("SKD-010")]
#[tokio::test]
async fn an_agent_started_with_a_kit_role_takes_no_other() {
    let a = agent();
    let orchestrator = AgentKeypair::generate();
    open(&a, true, None)
        .await
        .delegate(envelope(&orchestrator, &a.key, "writer").wire().clone())
        .await
        .unwrap();

    // A kit agent on the same host shares the key's state but not its role.
    let b = Agent {
        metadata: Arc::new(RwLock::new(AgentMetadata {
            purpose: "Kit purpose".into(),
            ..AgentMetadata::default()
        })),
        ..agent()
    };
    let kit_agent = SpecialistService::open(
        SpecialistConfig {
            key: a.key.clone(),
            state_dir: a.dir.clone(),
            enlistable: false,
            operator_orchestrator: None,
            resolve_model: resolver(),
        },
        b.metadata.clone(),
        None,
    )
    .await
    .unwrap();
    let status = kit_agent.status().await;
    assert!(status.binding.is_none() && status.delegation.is_none() && status.binds_by.is_empty());
    assert_eq!(b.metadata.read().await.purpose, "Kit purpose");
    let error = kit_agent
        .delegate(envelope(&orchestrator, &a.key, "writer").wire().clone())
        .await
        .unwrap_err();
    assert_eq!(reason(&error), RefusalReason::BindingRequired);
}

#[spec("SKD-006", "SKD-011")]
#[tokio::test]
async fn a_restarted_agent_resumes_its_role() {
    let a = agent();
    let orchestrator = AgentKeypair::generate();
    open(&a, true, None)
        .await
        .delegate(envelope(&orchestrator, &a.key, "writer").wire().clone())
        .await
        .unwrap();

    let restarted = agent();
    let service = SpecialistService::open(
        SpecialistConfig {
            key: a.key.clone(),
            state_dir: a.dir.clone(),
            enlistable: true,
            operator_orchestrator: None,
            resolve_model: resolver(),
        },
        restarted.metadata.clone(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(service.status().await.delegation.unwrap().role_id, "writer");
    assert_eq!(
        restarted.metadata.read().await.granted_tools,
        ["generate_image"]
    );
}

#[spec("SKD-007")]
#[tokio::test]
async fn the_operators_orchestrator_replaces_a_first_use_binding() {
    let a = agent();
    let (first, named) = (AgentKeypair::generate(), AgentKeypair::generate());
    open(&a, true, None)
        .await
        .delegate(envelope(&first, &a.key, "writer").wire().clone())
        .await
        .unwrap();
    let service = SpecialistService::open(
        SpecialistConfig {
            key: a.key.clone(),
            state_dir: a.dir.clone(),
            enlistable: true,
            operator_orchestrator: Some(named.public_key().to_did_key()),
            resolve_model: resolver(),
        },
        agent().metadata,
        None,
    )
    .await
    .unwrap();
    let status = service.status().await;
    let binding = status.binding.unwrap();
    assert_eq!(binding.mode, BindingMode::Operator);
    assert_eq!(binding.orchestrator, named.public_key().to_did_key());
    assert!(status.delegation.is_none());
    let error = service
        .delegate(envelope(&first, &a.key, "writer").wire().clone())
        .await
        .unwrap_err();
    assert_eq!(reason(&error), RefusalReason::NotBoundOrchestrator);
}

/// SKD-011, ASSET-010: a host that can generate serves generation only
/// while its role requires it.
#[spec("SKD-011", "ASSET-010")]
#[tokio::test]
async fn generation_follows_the_role() {
    let fake = arkavo_image_gen::testing::fake_generator(0, std::time::Duration::ZERO).await;
    let assets = Arc::new(AssetService::new(
        Some(fake.generator.clone()),
        None,
        transport().await,
    ));
    let a = agent();
    let service = open(&a, true, Some(assets.clone())).await;
    let capability = service.status().await.capabilities;
    assert_eq!(capability.len(), 1);
    assert_eq!(capability[0].name, IMAGE_GENERATION);
    assert!(!capability[0].ready, "no weights fetched yet");
    assert!(assets.descriptor().is_some(), "a zero-config agent serves");

    let orchestrator = AgentKeypair::generate();
    service
        .delegate(envelope(&orchestrator, &a.key, "writer").wire().clone())
        .await
        .unwrap();
    assert!(assets.descriptor().is_none(), "the writer asks the artist");
    assert_eq!(
        service.status().await.capabilities.len(),
        1,
        "the host still could"
    );

    service
        .delegate(envelope(&orchestrator, &a.key, "artist").wire().clone())
        .await
        .unwrap();
    assert!(assets.descriptor().is_some(), "the artist serves");
    drop(fake.server);
}
