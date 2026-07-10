//! End-to-end test: launch the Campaign Kit MVP manifest, drive a synthetic
//! pipeline (analyst → copy → critic), and verify each role's DecisionTrace
//! captures only its own events.
//!
//! This is the proof that the SwarmKit `agent_provisioning` → ARP handoff
//! described in spec §1.2 / §5 actually works end-to-end on a real
//! manifest from arkavo-org/arkavo-edge#573.

// #[tokio::test] expands to Runtime::block_on; harmless in tests.
#![allow(clippy::disallowed_methods)]

use arkavo_swarmkit::parse_yaml;
use arkavo_swarmkit_runtime::{LaunchOptions, SwarmFlight};
use arkavo_test_macros::spec;

const CAMPAIGN_KIT: &str =
    include_str!("../../../examples/campaign-kit/campaign-kit.swarmkit.yaml");

#[spec("SK-011")]
#[tokio::test]
async fn campaign_kit_flight_isolates_per_role_traces() {
    let manifest = parse_yaml(CAMPAIGN_KIT).expect("Campaign Kit manifest must validate");

    let flight =
        SwarmFlight::launch(&manifest, LaunchOptions::default()).expect("Campaign Kit must launch");

    // Sanity: 3 roles in the order the manifest declared them.
    let roles: Vec<&str> = flight.roles().map(|r| r.role_id()).collect();
    assert_eq!(roles, vec!["analyst", "copy", "critic"]);
    assert_eq!(flight.role("analyst").unwrap().role_type(), "asset_analyst");
    assert_eq!(flight.role("copy").unwrap().role_type(), "platform_copy");
    assert_eq!(flight.role("critic").unwrap().role_type(), "critic");

    // Drive the pipeline. Two analyst tools, two copy tools, one critic eval.
    flight
        .record_tool_outcome("analyst", "asset.summarize", true, 0.92)
        .await
        .unwrap();
    flight
        .record_tool_outcome("analyst", "asset.extract_selling_points", true, 0.88)
        .await
        .unwrap();
    flight
        .record_tool_outcome("copy", "copy.draft_post", true, 0.81)
        .await
        .unwrap();
    flight
        .record_tool_outcome("copy", "copy.refine_tone", true, 0.84)
        .await
        .unwrap();
    flight
        .record_tool_outcome("critic", "critic.score_rubric", true, 0.76)
        .await
        .unwrap();

    // Each role's trace contains only its own events.
    let analyst = flight.role("analyst").unwrap().arp().decision_trace();
    let copy = flight.role("copy").unwrap().arp().decision_trace();
    let critic = flight.role("critic").unwrap().arp().decision_trace();

    assert_eq!(analyst.len(), 2);
    assert_eq!(copy.len(), 2);
    assert_eq!(critic.len(), 1);

    let analyst_entries = analyst.snapshot();
    for entry in &analyst_entries {
        assert_eq!(entry.agent_id, "analyst");
        assert!(
            entry
                .decision
                .chosen
                .as_deref()
                .unwrap()
                .starts_with("asset.")
        );
    }

    let copy_entries = copy.snapshot();
    for entry in &copy_entries {
        assert_eq!(entry.agent_id, "copy");
        assert!(
            entry
                .decision
                .chosen
                .as_deref()
                .unwrap()
                .starts_with("copy.")
        );
    }

    let critic_entries = critic.snapshot();
    assert_eq!(critic_entries[0].agent_id, "critic");
    assert_eq!(
        critic_entries[0].decision.chosen.as_deref(),
        Some("critic.score_rubric")
    );

    // Each role's PolicyCache is independent — analyst tool entries never
    // appear in copy's cache.
    let analyst_keys: Vec<String> = flight
        .role("analyst")
        .unwrap()
        .arp()
        .cache()
        .snapshot()
        .into_iter()
        .map(|e| e.key)
        .collect();
    let copy_keys: Vec<String> = flight
        .role("copy")
        .unwrap()
        .arp()
        .cache()
        .snapshot()
        .into_iter()
        .map(|e| e.key)
        .collect();
    assert!(analyst_keys.iter().all(|k| k.contains("asset.")));
    assert!(copy_keys.iter().all(|k| k.contains("copy.")));
}

#[spec("SK-012")]
#[tokio::test]
async fn below_quality_gate_outcome_degrades_role_prior() {
    let manifest = parse_yaml(CAMPAIGN_KIT).unwrap();
    let flight = SwarmFlight::launch(&manifest, LaunchOptions::default()).unwrap();

    // Critic returns a low-quality score — below the derived 0.7 gate. The
    // ARP runtime must downgrade the prior.
    flight
        .record_tool_outcome("critic", "critic.score_rubric", true, 0.40)
        .await
        .unwrap();

    let critic_arp = flight.role("critic").unwrap().arp();
    let snap = critic_arp.adaptation().lock().await.snapshot();
    let prior = snap
        .iter()
        .find(|p| p.id == "critic.score_rubric")
        .expect("prior recorded");
    assert!(
        prior.beta > 1.0,
        "below-gate outcome should grow beta: {}",
        prior.beta
    );

    let entry = &critic_arp.decision_trace().snapshot()[0];
    assert_eq!(entry.outcome.success, Some(false));
    assert_eq!(
        entry.outcome.error_type.as_deref(),
        Some("below_quality_gate")
    );
}

#[spec("SK-012")]
#[tokio::test]
async fn above_quality_gate_outcome_keeps_prior_unchanged() {
    let manifest = parse_yaml(CAMPAIGN_KIT).unwrap();
    let flight = SwarmFlight::launch(&manifest, LaunchOptions::default()).unwrap();

    flight
        .record_tool_outcome("critic", "critic.score_rubric", true, 0.95)
        .await
        .unwrap();

    let critic_arp = flight.role("critic").unwrap().arp();
    let snap = critic_arp.adaptation().lock().await.snapshot();
    let prior = snap
        .iter()
        .find(|p| p.id == "critic.score_rubric")
        .expect("prior recorded");
    assert_eq!(
        prior.beta, 1.0,
        "above-gate outcome should not grow beta: {}",
        prior.beta
    );

    let entry = &critic_arp.decision_trace().snapshot()[0];
    assert_eq!(entry.outcome.success, Some(true));
    assert!(entry.outcome.error_type.is_none());
}
