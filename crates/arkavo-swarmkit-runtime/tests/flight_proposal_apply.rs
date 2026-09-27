//! Flight-radius proposal apply: atomicity and convergence (#609).

// #[tokio::test] expands to Runtime::block_on; harmless in tests.
#![allow(clippy::disallowed_methods)]

use arkavo_arp::proposal::{
    BlastRadius, ProposalOrigin, ProposalState, TighteningEffect, TighteningProposal, TraceRef,
};
use arkavo_swarmkit::parse_yaml;
use arkavo_swarmkit_runtime::flight::{LaunchOptions, SwarmFlight};
use arkavo_swarmkit_runtime::flight_apply::{
    FlightApplyError, apply_flight_proposal, reconcile_flight_proposal,
};
use arkavo_test_macros::spec;

const KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "proposal-apply-kit"
  version: "1.0.0"
  authors: [{did: "did:web:example.com"}]
  created: "2026-06-01T00:00:00Z"
  expires: "2026-06-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "exercise flight-radius proposal apply"
roles:
  - {id: "commander", role_type: "coordinator", agent_provisioning: {}}
  - {id: "survival", role_type: "specialist", agent_provisioning: {}}
  - {id: "historian", role_type: "auditor", plane: "audit", agent_provisioning: {}}
coordination:
  topology: "hub-spoke"
  protocol: "a2a-jsonrpc-2.0"
  routing: {strategy: "static"}
constraints:
  global_budget: {max_wallclock_seconds: 60, max_total_tokens: 8000, max_cost_usd: 0.30}
  data_classifications: ["public"]
  network: {egress_allowed: false, egress_allowlist: []}
proposal_governance:
  accepted_origins: ["consolidation", "human"]
  role_ceilings:
    - {role: "commander", auto_apply_max_blast_radius: "single_agent"}
  observation_window_sec: 600
completion:
  rules: ["all deliverables present"]
  on_failure: "abort"
  max_retries: 0
provenance:
  signatures: [{signer_did: "did:web:example.com", algorithm: "ed25519", signature: "AAA"}]
"#;

fn launch() -> SwarmFlight {
    let manifest = parse_yaml(KIT).unwrap();
    SwarmFlight::launch(&manifest, LaunchOptions::default()).unwrap()
}

fn flight_proposal(id: &str, effect: TighteningEffect) -> TighteningProposal {
    TighteningProposal {
        id: id.into(),
        origin: ProposalOrigin::Consolidation,
        state: ProposalState::Proposed,
        effect,
        rationale: "flight-wide tightening from nightly consolidation".into(),
        blast_radius: BlastRadius::Flight,
        evidence: vec![TraceRef {
            trace_id: "t-1".into(),
            episode_ids: vec!["ep-1".into()],
        }],
        created_at: "2026-06-10T00:00:00Z".into(),
        applied_at: None,
        reviewed_by: None,
        disposition_reason: None,
    }
}

#[spec("SK-094")]
#[tokio::test]
async fn flight_apply_reaches_every_role() {
    let flight = launch();
    let p = flight_proposal(
        "fp-1",
        TighteningEffect::DenyTool {
            tool: "game-rl:reset".into(),
        },
    );
    apply_flight_proposal(&flight, &p, "kit-approver", 1_000)
        .await
        .unwrap();

    for role in flight.roles() {
        let queue = role.arp().proposal_queue();
        let guard = queue.lock().await;
        assert_eq!(
            guard.proposal_state("fp-1"),
            Some(ProposalState::Observed),
            "role {} should be observing the applied proposal",
            role.role_id()
        );
        assert!(guard.is_tool_denied("game-rl:reset"));
    }
}

#[spec("SK-094")]
#[spec("ARP-006")]
#[tokio::test]
async fn mesh_radius_is_refused_outright() {
    let flight = launch();
    let mut p = flight_proposal("mp-1", TighteningEffect::DenyTool { tool: "x".into() });
    p.blast_radius = BlastRadius::Mesh;
    let err = apply_flight_proposal(&flight, &p, "kit-approver", 1_000)
        .await
        .unwrap_err();
    assert_eq!(err, FlightApplyError::WrongRadius(BlastRadius::Mesh));
    for role in flight.roles() {
        let queue = role.arp().proposal_queue();
        assert!(queue.lock().await.proposal_state("mp-1").is_none());
    }
}

#[spec("SK-094")]
#[tokio::test]
async fn precheck_failure_applies_nothing_anywhere() {
    let flight = launch();
    // Pre-deny the tool on one role only: the flight-wide apply must now
    // fail admission there and leave every role untouched.
    {
        let role = flight.role("survival").unwrap();
        let queue = role.arp().proposal_queue();
        queue
            .lock()
            .await
            .ingest_reviewed(
                {
                    let mut p = flight_proposal(
                        "pre-existing",
                        TighteningEffect::DenyTool {
                            tool: "game-rl:reset".into(),
                        },
                    );
                    p.blast_radius = BlastRadius::SingleEntity;
                    p
                },
                "operator",
                500,
            )
            .unwrap();
    }

    let p = flight_proposal(
        "fp-2",
        TighteningEffect::DenyTool {
            tool: "game-rl:reset".into(),
        },
    );
    let err = apply_flight_proposal(&flight, &p, "kit-approver", 1_000)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        FlightApplyError::PrecheckFailed { ref role_id, .. } if role_id == "survival"
    ));
    // Atomicity: no role carries fp-2 — including roles checked before the
    // failing one.
    for role in flight.roles() {
        let queue = role.arp().proposal_queue();
        assert!(queue.lock().await.proposal_state("fp-2").is_none());
    }
}

#[spec("SK-095")]
#[tokio::test]
async fn kill_one_role_mid_apply_reconciles_to_convergence() {
    let flight = launch();
    // Simulate a crash between per-role applies: the proposal landed on
    // commander and survival, then the process died before historian.
    let p = flight_proposal(
        "fp-3",
        TighteningEffect::DenyTool {
            tool: "game-rl:reset".into(),
        },
    );
    for role_id in ["commander", "survival"] {
        let role = flight.role(role_id).unwrap();
        let queue = role.arp().proposal_queue();
        queue
            .lock()
            .await
            .ingest_reviewed(p.clone(), "kit-approver", 1_000)
            .unwrap();
    }

    // Recovery: reconcile detects the partial application and reverts it
    // from the roles that have it.
    let reverted = reconcile_flight_proposal(&flight, "fp-3").await;
    assert_eq!(
        reverted,
        vec!["commander".to_string(), "survival".to_string()]
    );

    for role in flight.roles() {
        let queue = role.arp().proposal_queue();
        let guard = queue.lock().await;
        // Converged: nobody enforces the partial proposal.
        assert!(!guard.is_tool_denied("game-rl:reset"));
        // And on the roles that had applied it, the revert is auditable.
        if ["commander", "survival"].contains(&role.role_id()) {
            assert_eq!(guard.proposal_state("fp-3"), Some(ProposalState::Reverted));
        }
    }

    // Idempotence: a second reconcile is a no-op.
    assert!(reconcile_flight_proposal(&flight, "fp-3").await.is_empty());
}

#[spec("SK-095")]
#[tokio::test]
async fn reconcile_reverts_a_confirmed_role_to_converge() {
    // Regression (#609): a delayed reconcile can find a role whose proposal
    // already passed its observation window and confirmed, while a peer never
    // received it. Reconcile must still revert the confirmed role — otherwise
    // it stays permanently tightened and the flight never converges.
    let flight = launch();
    let p = flight_proposal(
        "fp-5",
        TighteningEffect::DenyTool {
            tool: "game-rl:reset".into(),
        },
    );
    // commander + survival applied the proposal and then confirmed it (clean
    // observation window elapsed); historian crashed before receiving it.
    for role_id in ["commander", "survival"] {
        let role = flight.role(role_id).unwrap();
        let queue = role.arp().proposal_queue();
        let mut guard = queue.lock().await;
        guard
            .ingest_reviewed(p.clone(), "kit-approver", 1_000)
            .unwrap();
        for _ in 0..10 {
            guard.record_quality_gate(true);
        }
        let transitions = guard.evaluate_observations(1_700);
        assert_eq!(transitions, vec![("fp-5".into(), ProposalState::Confirmed)]);
    }

    let reverted = reconcile_flight_proposal(&flight, "fp-5").await;
    assert_eq!(
        reverted,
        vec!["commander".to_string(), "survival".to_string()]
    );
    for role in flight.roles() {
        let queue = role.arp().proposal_queue();
        let guard = queue.lock().await;
        // Converged: the confirmed tightening was undone everywhere.
        assert!(!guard.is_tool_denied("game-rl:reset"));
        if ["commander", "survival"].contains(&role.role_id()) {
            assert_eq!(guard.proposal_state("fp-5"), Some(ProposalState::Reverted));
        }
    }
    // Idempotent: a second reconcile finds nothing left to revert.
    assert!(reconcile_flight_proposal(&flight, "fp-5").await.is_empty());
}

#[spec("SK-095")]
#[tokio::test]
async fn converged_flight_reconciles_as_noop() {
    let flight = launch();
    let p = flight_proposal(
        "fp-4",
        TighteningEffect::DenyTool {
            tool: "game-rl:reset".into(),
        },
    );
    apply_flight_proposal(&flight, &p, "kit-approver", 1_000)
        .await
        .unwrap();
    assert!(reconcile_flight_proposal(&flight, "fp-4").await.is_empty());
    for role in flight.roles() {
        let queue = role.arp().proposal_queue();
        assert!(queue.lock().await.is_tool_denied("game-rl:reset"));
    }
}
