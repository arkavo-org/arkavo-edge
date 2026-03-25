//! Integration test: parse the fleet-immunity rover-alpha ARP document.

#[test]
fn parse_rover_alpha_arp() {
    let json = include_str!("../../../examples/fleet-immunity/rover-alpha.arp.json");
    let doc = arkavo_arp::parse(json).expect("rover-alpha.arp.json should be a valid ARP document");

    assert_eq!(doc.arp_spec, "0.1.0");
    assert_eq!(
        doc.adl_ref.uri.as_deref(),
        Some("file://./rover-alpha.adl.json")
    );

    // Adaptation: Thompson Sampling with optimistic cold start
    assert_eq!(
        doc.adaptation.method,
        arkavo_arp::adaptation::AdaptationMethod::ThompsonSampling
    );
    let cold_start = doc.adaptation.cold_start.as_ref().expect("cold_start");
    assert_eq!(
        cold_start.strategy,
        Some(arkavo_arp::adaptation::ColdStartStrategy::Optimistic)
    );
    assert_eq!(cold_start.warmup_period, Some(3));

    // Feedback: quality gate with overrides
    let gate = &doc.feedback_loops.immediate.quality_gate;
    assert_eq!(gate.threshold_default, 0.6);
    let overrides = gate.threshold_overrides.as_ref().expect("overrides");
    assert_eq!(overrides.len(), 2);
    assert_eq!(overrides[0].task_class, "hazard_detection");
    assert_eq!(overrides[0].threshold, 0.8);

    // PolicyCache: exponential decay, 5-minute half-life
    let cache = &doc.feedback_loops.short_term.policy_cache;
    assert_eq!(
        cache.decay_strategy,
        arkavo_arp::feedback::DecayStrategy::Exponential
    );
    assert_eq!(cache.decay_half_life_sec, Some(300));

    // Gossip: quorum of 2 trusted peers
    let gossip = doc.feedback_loops.gossip.as_ref().expect("gossip");
    assert_eq!(gossip.quorum.min_trusted_peers, 2);
    assert!(gossip.require_did_signature);

    // Budget: $0.10 ceiling, 30 tool calls/min
    assert_eq!(doc.budget.task_ceiling_usd, 0.10);
    assert_eq!(doc.budget.velocity.max_tool_calls_per_minute, Some(30));

    // Tool risk: get_sector is low, inject_hazard is high
    let exec = doc.execution.as_ref().expect("execution");
    let classifications = exec
        .tool_risk
        .as_ref()
        .expect("tool_risk")
        .classifications
        .as_ref()
        .expect("classifications");
    assert_eq!(classifications[0].tool_pattern, "get_sector");
    assert_eq!(classifications[0].risk, "low");
    assert_eq!(classifications[1].tool_pattern, "inject_hazard");
    assert_eq!(classifications[1].risk, "high");

    // Escalation: one-way ratchet
    let esc = doc.escalation.as_ref().expect("escalation");
    assert_eq!(esc.ratchet_direction.as_deref(), Some("tighten_only"));
    assert_eq!(esc.relaxation_requires.as_deref(), Some("human_approval"));
    let rules = esc.cross_layer_rules.as_ref().expect("rules");
    assert_eq!(rules.len(), 3);

    // Session: 10-minute absolute, 2-minute idle
    let session = doc.session.as_ref().expect("session");
    let defaults = session.defaults.expect("defaults");
    assert_eq!(defaults.absolute_timeout_sec, Some(600));
    assert_eq!(defaults.idle_timeout_sec, Some(120));

    // DecisionTrace: enabled, Ed25519 signing
    let obs = doc.observability.as_ref().expect("observability");
    let trace = obs.decision_trace.as_ref().expect("decision_trace");
    assert_eq!(trace.enabled, Some(true));
    let signing = trace.cryptographic_signing.as_ref().expect("signing");
    assert_eq!(signing.algorithm.as_deref(), Some("Ed25519"));
}
