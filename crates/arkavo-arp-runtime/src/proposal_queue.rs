//! Tightening-proposal queue and lifecycle state machine (§18).
//!
//! States: `proposed → reviewed → applied → observed → confirmed | reverted`,
//! with `rejected` reachable at ingest or review. Auto-apply happens when the
//! proposal's blast radius is within the document's
//! `proposal_policy.auto_apply_max_blast_radius`; everything wider waits for
//! HITL review. Applying an effect records the prior value; a revert restores
//! it AND emits a `proposal_lifecycle` DecisionTrace entry — reverts are
//! auditable, never erased.
//!
//! Observation semantics (v1): while a proposal is `observed`, quality-gate
//! outcomes are tallied; at window end the in-window failure rate is compared
//! against the pre-apply baseline. A regression beyond
//! [`REGRESSION_MARGIN`] reverts the proposal, otherwise it is confirmed.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arkavo_arp::ArpDocument;
use arkavo_arp::constraints::{LayerBudget, PerLayerBudget};
use arkavo_arp::observability::{TraceDecision, TraceEventType, TraceLayer, TraceOutcome};
use arkavo_arp::proposal::{
    BudgetLayerName, ProposalPolicy, ProposalState, TighteningEffect, TighteningProposal,
};
use arkavo_arp::validate::validate_proposal;
use arkavo_observability::decision_trace::DecisionTrace;

/// In-window failure rate may exceed the pre-apply baseline by this margin
/// before the change is considered a regression.
pub const REGRESSION_MARGIN: f64 = 0.05;

/// Value displaced by an applied effect, kept so a revert can restore it.
#[derive(Debug, Clone, PartialEq)]
enum DisplacedValue {
    TaskCeiling(f64),
    LayerBudget(BudgetLayerName, Option<f64>),
    QualityThreshold(f64),
    /// Set-insertion effects (quarantine/deny): revert removes the key.
    SetInsertion,
    RateLimit(Option<f64>),
}

/// Per-applied-proposal observation bookkeeping.
#[derive(Debug, Clone)]
struct Observation {
    started_at_epoch_sec: u64,
    window_sec: u64,
    baseline_failure_rate: f64,
    checks: u64,
    failures: u64,
    displaced: DisplacedValue,
}

/// Queue of tightening proposals plus the live policy document their effects
/// apply to. The queue owns the canonical post-apply document; accessors
/// expose the effective values and deny sets to the conductor.
pub struct ProposalQueue {
    policy: Option<ProposalPolicy>,
    doc: ArpDocument,
    proposals: Vec<TighteningProposal>,
    observations: HashMap<String, Observation>,
    denied_tools: HashSet<String>,
    denied_hosts: HashSet<String>,
    /// `"{scope:?}:{entity}"` keys for applied quarantines.
    quarantined: HashSet<String>,
    gate_checks: u64,
    gate_failures: u64,
    trace: Arc<DecisionTrace>,
    agent_id: String,
}

impl ProposalQueue {
    /// Build a queue around a document. The policy comes from
    /// `doc.proposal_policy`; without one, every ingest is rejected.
    pub fn new(doc: ArpDocument, trace: Arc<DecisionTrace>, agent_id: String) -> Self {
        Self {
            policy: doc.proposal_policy.clone(),
            doc,
            proposals: Vec::new(),
            observations: HashMap::new(),
            denied_tools: HashSet::new(),
            denied_hosts: HashSet::new(),
            quarantined: HashSet::new(),
            gate_checks: 0,
            gate_failures: 0,
            trace,
            agent_id,
        }
    }

    /// The current effective document (post-applied effects).
    pub fn document(&self) -> &ArpDocument {
        &self.doc
    }

    /// Snapshot of all proposals and their states.
    pub fn proposals(&self) -> Vec<TighteningProposal> {
        self.proposals.clone()
    }

    pub fn is_tool_denied(&self, tool: &str) -> bool {
        self.denied_tools.contains(tool)
    }

    pub fn is_host_denied(&self, host: &str) -> bool {
        self.denied_hosts.contains(host)
    }

    /// Ingest a proposal: validate, direction-check against current values,
    /// then either auto-apply (within policy radius) or hold for review.
    /// Returns the resulting state; rejects are recorded, not dropped.
    pub fn ingest(
        &mut self,
        mut proposal: TighteningProposal,
        now_epoch_sec: u64,
    ) -> ProposalState {
        let Some(policy) = self.policy.clone() else {
            return self.reject(proposal, "document has no proposal_policy");
        };
        if let Err(e) = validate_proposal(&proposal, &policy) {
            let reason = e.to_string();
            return self.reject(proposal, &reason);
        }
        if let Err(reason) = self.direction_violation(&proposal.effect) {
            return self.reject(proposal, &reason);
        }

        proposal.state = ProposalState::Proposed;
        if proposal.blast_radius <= policy.auto_apply_max_blast_radius {
            let window = policy.observation_window_sec.unwrap_or(3600);
            self.apply(&mut proposal, now_epoch_sec, window);
        }
        let state = proposal.state;
        self.proposals.push(proposal);
        state
    }

    /// HITL review of a held proposal: approve applies it, deny rejects it.
    pub fn review(
        &mut self,
        proposal_id: &str,
        approve: bool,
        reviewer: &str,
        now_epoch_sec: u64,
    ) -> Result<ProposalState, String> {
        let window = self
            .policy
            .as_ref()
            .and_then(|p| p.observation_window_sec)
            .unwrap_or(3600);
        let idx = self
            .proposals
            .iter()
            .position(|p| p.id == proposal_id)
            .ok_or_else(|| format!("unknown proposal {proposal_id}"))?;
        if self.proposals[idx].state != ProposalState::Proposed {
            return Err(format!(
                "proposal {proposal_id} is {:?}, not awaiting review",
                self.proposals[idx].state
            ));
        }

        let mut proposal = self.proposals[idx].clone();
        proposal.reviewed_by = Some(reviewer.to_string());
        proposal.state = ProposalState::Reviewed;
        if approve {
            // Re-check direction: current values may have moved since ingest.
            if let Err(reason) = self.direction_violation(&proposal.effect) {
                proposal.state = ProposalState::Rejected;
                proposal.disposition_reason = Some(reason);
            } else {
                self.apply(&mut proposal, now_epoch_sec, window);
            }
        } else {
            proposal.state = ProposalState::Rejected;
            proposal.disposition_reason = Some(format!("rejected by reviewer {reviewer}"));
        }
        let state = proposal.state;
        self.proposals[idx] = proposal;
        Ok(state)
    }

    /// Feed a quality-gate outcome into the baseline and all active windows.
    pub fn record_quality_gate(&mut self, passed: bool) {
        self.gate_checks += 1;
        if !passed {
            self.gate_failures += 1;
        }
        for obs in self.observations.values_mut() {
            obs.checks += 1;
            if !passed {
                obs.failures += 1;
            }
        }
    }

    /// Evaluate all observation windows: confirm proposals whose window
    /// elapsed without regression, revert those that regressed. Returns the
    /// transitions made.
    pub fn evaluate_observations(&mut self, now_epoch_sec: u64) -> Vec<(String, ProposalState)> {
        let due: Vec<String> = self
            .observations
            .iter()
            .filter(|(_, obs)| now_epoch_sec >= obs.started_at_epoch_sec + obs.window_sec)
            .map(|(id, _)| id.clone())
            .collect();

        let mut transitions = Vec::new();
        for id in due {
            let obs = self.observations.remove(&id).expect("due id exists");
            let window_rate = if obs.checks == 0 {
                0.0
            } else {
                obs.failures as f64 / obs.checks as f64
            };
            let regressed = window_rate > obs.baseline_failure_rate + REGRESSION_MARGIN;

            let idx = self.proposals.iter().position(|p| p.id == id);
            let Some(idx) = idx else { continue };
            if regressed {
                let reason = format!(
                    "quality-gate regression during observation: window failure rate {:.3} \
                     vs baseline {:.3}",
                    window_rate, obs.baseline_failure_rate
                );
                let effect = self.proposals[idx].effect.clone();
                self.restore(&effect, &obs.displaced);
                self.proposals[idx].state = ProposalState::Reverted;
                self.proposals[idx].disposition_reason = Some(reason.clone());
                self.emit_trace(&self.proposals[idx].clone(), "reverted", Some(reason));
                transitions.push((id, ProposalState::Reverted));
            } else {
                self.proposals[idx].state = ProposalState::Confirmed;
                self.emit_trace(&self.proposals[idx].clone(), "confirmed", None);
                transitions.push((id, ProposalState::Confirmed));
            }
        }
        transitions
    }

    fn reject(&mut self, mut proposal: TighteningProposal, reason: &str) -> ProposalState {
        proposal.state = ProposalState::Rejected;
        proposal.disposition_reason = Some(reason.to_string());
        self.proposals.push(proposal);
        ProposalState::Rejected
    }

    /// Direction check against *current* values: the typed effect already
    /// guarantees the tighten direction by construction; this verifies the
    /// new value is actually stricter than what is in force right now.
    fn direction_violation(&self, effect: &TighteningEffect) -> Result<(), String> {
        match effect {
            TighteningEffect::LowerTaskCeiling { new_ceiling_usd } => {
                let current = self.doc.budget.task_ceiling_usd;
                if *new_ceiling_usd >= current {
                    return Err(format!(
                        "new ceiling {new_ceiling_usd} does not tighten current {current}"
                    ));
                }
            }
            TighteningEffect::LowerLayerBudget {
                layer,
                new_limit_usd,
            } => {
                if *layer == BudgetLayerName::Consolidation {
                    return Err(
                        "consolidation layer budget is not wired into per_layer yet".to_string()
                    );
                }
                if let Some(current) = self.layer_limit(*layer)
                    && *new_limit_usd >= current
                {
                    return Err(format!(
                        "new layer limit {new_limit_usd} does not tighten current {current}"
                    ));
                }
                // No current limit: introducing one is a tightening.
            }
            TighteningEffect::RaiseQualityGateThreshold { new_threshold } => {
                let current = self
                    .doc
                    .feedback_loops
                    .immediate
                    .quality_gate
                    .threshold_default;
                if *new_threshold <= current {
                    return Err(format!(
                        "new threshold {new_threshold} does not tighten current {current}"
                    ));
                }
            }
            TighteningEffect::AddQuarantine { scope, entity } => {
                if self.quarantined.contains(&format!("{scope:?}:{entity}")) {
                    return Err(format!("{entity} is already quarantined"));
                }
            }
            TighteningEffect::DenyTool { tool } => {
                if self.denied_tools.contains(tool) {
                    return Err(format!("tool {tool} is already denied"));
                }
            }
            TighteningEffect::DenyEgress { host } => {
                if self.denied_hosts.contains(host) {
                    return Err(format!("host {host} is already denied"));
                }
            }
            TighteningEffect::LowerRateLimit {
                new_requests_per_second,
            } => {
                let current = self
                    .doc
                    .budget
                    .rate_limiting
                    .as_ref()
                    .and_then(|r| r.global.as_ref())
                    .and_then(|g| g.requests_per_second);
                if let Some(current) = current
                    && *new_requests_per_second >= current
                {
                    return Err(format!(
                        "new rate {new_requests_per_second} does not tighten current {current}"
                    ));
                }
            }
        }
        Ok(())
    }

    fn layer_limit(&self, layer: BudgetLayerName) -> Option<f64> {
        let per = self.doc.budget.per_layer.as_ref()?;
        let lb = match layer {
            BudgetLayerName::Cognitive => per.cognitive,
            BudgetLayerName::Execution => per.execution,
            BudgetLayerName::Data => per.data,
            BudgetLayerName::Network => per.network,
            BudgetLayerName::Consolidation => None,
        };
        lb.and_then(|l| l.limit_usd)
    }

    /// Apply an effect to the live document/deny sets, recording the
    /// displaced value, and move the proposal to `observed`.
    fn apply(&mut self, proposal: &mut TighteningProposal, now_epoch_sec: u64, window_sec: u64) {
        let displaced = match &proposal.effect {
            TighteningEffect::LowerTaskCeiling { new_ceiling_usd } => {
                let old = self.doc.budget.task_ceiling_usd;
                self.doc.budget.task_ceiling_usd = *new_ceiling_usd;
                DisplacedValue::TaskCeiling(old)
            }
            TighteningEffect::LowerLayerBudget {
                layer,
                new_limit_usd,
            } => {
                let old = self.layer_limit(*layer);
                self.set_layer_limit(*layer, *new_limit_usd);
                DisplacedValue::LayerBudget(*layer, old)
            }
            TighteningEffect::RaiseQualityGateThreshold { new_threshold } => {
                let gate = &mut self.doc.feedback_loops.immediate.quality_gate;
                let old = gate.threshold_default;
                gate.threshold_default = *new_threshold;
                DisplacedValue::QualityThreshold(old)
            }
            TighteningEffect::AddQuarantine { scope, entity } => {
                self.quarantined.insert(format!("{scope:?}:{entity}"));
                DisplacedValue::SetInsertion
            }
            TighteningEffect::DenyTool { tool } => {
                self.denied_tools.insert(tool.clone());
                DisplacedValue::SetInsertion
            }
            TighteningEffect::DenyEgress { host } => {
                self.denied_hosts.insert(host.clone());
                DisplacedValue::SetInsertion
            }
            TighteningEffect::LowerRateLimit {
                new_requests_per_second,
            } => {
                let old = self
                    .doc
                    .budget
                    .rate_limiting
                    .as_ref()
                    .and_then(|r| r.global.as_ref())
                    .and_then(|g| g.requests_per_second);
                self.set_rate_limit(*new_requests_per_second);
                DisplacedValue::RateLimit(old)
            }
        };

        let baseline = if self.gate_checks == 0 {
            0.0
        } else {
            self.gate_failures as f64 / self.gate_checks as f64
        };
        self.observations.insert(
            proposal.id.clone(),
            Observation {
                started_at_epoch_sec: now_epoch_sec,
                window_sec,
                baseline_failure_rate: baseline,
                checks: 0,
                failures: 0,
                displaced,
            },
        );
        proposal.state = ProposalState::Observed;
        proposal.applied_at = Some(format_epoch(now_epoch_sec));
        self.emit_trace(proposal, "applied", None);
    }

    /// Restore a displaced value — the revert path.
    fn restore(&mut self, effect: &TighteningEffect, displaced: &DisplacedValue) {
        match (effect, displaced) {
            (_, DisplacedValue::TaskCeiling(old)) => {
                self.doc.budget.task_ceiling_usd = *old;
            }
            (_, DisplacedValue::LayerBudget(layer, old)) => match old {
                Some(v) => self.set_layer_limit(*layer, *v),
                None => self.clear_layer_limit(*layer),
            },
            (_, DisplacedValue::QualityThreshold(old)) => {
                self.doc
                    .feedback_loops
                    .immediate
                    .quality_gate
                    .threshold_default = *old;
            }
            (TighteningEffect::AddQuarantine { scope, entity }, DisplacedValue::SetInsertion) => {
                self.quarantined.remove(&format!("{scope:?}:{entity}"));
            }
            (TighteningEffect::DenyTool { tool }, DisplacedValue::SetInsertion) => {
                self.denied_tools.remove(tool);
            }
            (TighteningEffect::DenyEgress { host }, DisplacedValue::SetInsertion) => {
                self.denied_hosts.remove(host);
            }
            (_, DisplacedValue::RateLimit(old)) => match old {
                Some(v) => self.set_rate_limit(*v),
                None => {
                    if let Some(r) = self.doc.budget.rate_limiting.as_mut()
                        && let Some(g) = r.global.as_mut()
                    {
                        g.requests_per_second = None;
                    }
                }
            },
            // Effect/displaced mismatch cannot be constructed by apply().
            (_, DisplacedValue::SetInsertion) => {}
        }
    }

    fn set_layer_limit(&mut self, layer: BudgetLayerName, limit: f64) {
        let per = self.doc.budget.per_layer.get_or_insert(PerLayerBudget {
            cognitive: None,
            execution: None,
            data: None,
            network: None,
        });
        let slot = match layer {
            BudgetLayerName::Cognitive => &mut per.cognitive,
            BudgetLayerName::Execution => &mut per.execution,
            BudgetLayerName::Data => &mut per.data,
            BudgetLayerName::Network => &mut per.network,
            BudgetLayerName::Consolidation => return,
        };
        *slot = Some(LayerBudget {
            limit_usd: Some(limit),
        });
    }

    fn clear_layer_limit(&mut self, layer: BudgetLayerName) {
        if let Some(per) = self.doc.budget.per_layer.as_mut() {
            let slot = match layer {
                BudgetLayerName::Cognitive => &mut per.cognitive,
                BudgetLayerName::Execution => &mut per.execution,
                BudgetLayerName::Data => &mut per.data,
                BudgetLayerName::Network => &mut per.network,
                BudgetLayerName::Consolidation => return,
            };
            *slot = None;
        }
    }

    fn set_rate_limit(&mut self, rps: f64) {
        use arkavo_arp::constraints::{RateLimit, RateLimiting};
        let limiting = self.doc.budget.rate_limiting.get_or_insert(RateLimiting {
            global: None,
            per_endpoint: None,
            per_tool: None,
            max_tracking_entries: None,
            tracking_entry_ttl_sec: None,
            cleanup_interval_sec: None,
        });
        let global = limiting.global.get_or_insert(RateLimit {
            requests_per_second: None,
            burst_size: None,
        });
        global.requests_per_second = Some(rps);
    }

    fn emit_trace(&self, proposal: &TighteningProposal, transition: &str, reason: Option<String>) {
        let decision = TraceDecision {
            chosen: Some(format!("proposal {} {}", proposal.id, transition)),
            alternatives_considered: None,
            selection_method: None,
            prior_state: None,
            posterior_state: None,
        };
        let outcome = TraceOutcome {
            success: Some(transition != "reverted"),
            quality_score: None,
            latency_ms: None,
            cost_usd: None,
            error_type: reason,
        };
        self.trace.record(
            TraceLayer::Execution,
            TraceEventType::ProposalLifecycle,
            uuid::Uuid::nil(),
            &self.agent_id,
            decision,
            outcome,
        );
    }
}

fn format_epoch(epoch_sec: u64) -> String {
    chrono::DateTime::from_timestamp(epoch_sec as i64, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| format!("epoch:{epoch_sec}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_arp::constraints::QuarantineScope;
    use arkavo_arp::observability::DecisionTraceConfig;
    use arkavo_arp::proposal::{BlastRadius, ProposalOrigin, TraceRef};

    const DOC: &str = r#"{
        "arp_spec": "0.1.0",
        "adl_ref": {"uri": "https://example.com/adl.json"},
        "adaptation": {"method": "thompson_sampling"},
        "feedback_loops": {
            "immediate": {
                "quality_gate": {
                    "threshold_default": 0.7,
                    "metric": "cosine_similarity",
                    "on_failure": "update_prior_and_log"
                }
            },
            "short_term": {
                "policy_cache": {
                    "default_ttl_sec": 3600,
                    "decay_strategy": "linear"
                }
            }
        },
        "budget": {
            "task_ceiling_usd": 2.5,
            "on_exhaustion": "halt_and_report",
            "velocity": {"max_spend_per_minute_usd": 0.5}
        },
        "proposal_policy": {
            "accepted_origins": ["consolidation", "human"],
            "auto_apply_max_blast_radius": "single_entity",
            "observation_window_sec": 600
        }
    }"#;

    fn queue() -> ProposalQueue {
        let doc = arkavo_arp::parse(DOC).unwrap();
        let trace = Arc::new(DecisionTrace::new(DecisionTraceConfig {
            enabled: Some(true),
            storage: Some("append_only".into()),
            retention_days: None,
            cryptographic_signing: None,
        }));
        ProposalQueue::new(doc, trace, "test-agent".into())
    }

    fn proposal(id: &str, effect: TighteningEffect, radius: BlastRadius) -> TighteningProposal {
        TighteningProposal {
            id: id.into(),
            origin: ProposalOrigin::Consolidation,
            state: ProposalState::Proposed,
            effect,
            rationale: "evidence-backed".into(),
            blast_radius: radius,
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

    #[test]
    fn auto_apply_within_radius_goes_to_observed() {
        let mut q = queue();
        let state = q.ingest(
            proposal(
                "p1",
                TighteningEffect::DenyTool {
                    tool: "game-rl:reset".into(),
                },
                BlastRadius::SingleEntity,
            ),
            1_000,
        );
        assert_eq!(state, ProposalState::Observed);
        assert!(q.is_tool_denied("game-rl:reset"));
        // Apply emitted a proposal_lifecycle trace entry.
        assert_eq!(q.trace.len(), 1);
    }

    #[test]
    fn wider_radius_waits_for_review_then_applies() {
        let mut q = queue();
        let state = q.ingest(
            proposal(
                "p1",
                TighteningEffect::LowerTaskCeiling {
                    new_ceiling_usd: 1.0,
                },
                BlastRadius::SingleAgent,
            ),
            1_000,
        );
        assert_eq!(state, ProposalState::Proposed);
        assert!((q.document().budget.task_ceiling_usd - 2.5).abs() < 1e-9);

        let state = q.review("p1", true, "operator", 1_100).unwrap();
        assert_eq!(state, ProposalState::Observed);
        assert!((q.document().budget.task_ceiling_usd - 1.0).abs() < 1e-9);
        let p = &q.proposals()[0];
        assert_eq!(p.reviewed_by.as_deref(), Some("operator"));
    }

    #[test]
    fn reviewer_denial_rejects() {
        let mut q = queue();
        q.ingest(
            proposal(
                "p1",
                TighteningEffect::LowerTaskCeiling {
                    new_ceiling_usd: 1.0,
                },
                BlastRadius::SingleAgent,
            ),
            1_000,
        );
        let state = q.review("p1", false, "operator", 1_100).unwrap();
        assert_eq!(state, ProposalState::Rejected);
        assert!((q.document().budget.task_ceiling_usd - 2.5).abs() < 1e-9);
    }

    #[test]
    fn ingest_rejects_empty_evidence_bad_origin_and_non_tightening() {
        let mut q = queue();
        // Empty evidence.
        let mut p = proposal(
            "p1",
            TighteningEffect::DenyTool { tool: "x".into() },
            BlastRadius::SingleEntity,
        );
        p.evidence.clear();
        assert_eq!(q.ingest(p, 0), ProposalState::Rejected);

        // Unaccepted origin.
        let mut p = proposal(
            "p2",
            TighteningEffect::DenyTool { tool: "x".into() },
            BlastRadius::SingleEntity,
        );
        p.origin = ProposalOrigin::Critic;
        assert_eq!(q.ingest(p, 0), ProposalState::Rejected);

        // Direction violation: "lower" ceiling above the current one.
        let p = proposal(
            "p3",
            TighteningEffect::LowerTaskCeiling {
                new_ceiling_usd: 99.0,
            },
            BlastRadius::SingleEntity,
        );
        assert_eq!(q.ingest(p, 0), ProposalState::Rejected);
        // Every reject is recorded with a reason, never dropped.
        assert_eq!(q.proposals().len(), 3);
        assert!(q.proposals().iter().all(|p| p.disposition_reason.is_some()));
    }

    #[test]
    fn clean_window_confirms() {
        let mut q = queue();
        q.ingest(
            proposal(
                "p1",
                TighteningEffect::RaiseQualityGateThreshold { new_threshold: 0.8 },
                BlastRadius::SingleEntity,
            ),
            1_000,
        );
        for _ in 0..10 {
            q.record_quality_gate(true);
        }
        // Window (600s) not yet elapsed: no transition.
        assert!(q.evaluate_observations(1_300).is_empty());
        let transitions = q.evaluate_observations(1_700);
        assert_eq!(transitions, vec![("p1".into(), ProposalState::Confirmed)]);
        // Effect stays in force.
        assert!(
            (q.document()
                .feedback_loops
                .immediate
                .quality_gate
                .threshold_default
                - 0.8)
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn regression_reverts_and_restores_prior_value() {
        let mut q = queue();
        // Establish a clean baseline before the proposal.
        for _ in 0..10 {
            q.record_quality_gate(true);
        }
        q.ingest(
            proposal(
                "p1",
                TighteningEffect::RaiseQualityGateThreshold { new_threshold: 0.9 },
                BlastRadius::SingleEntity,
            ),
            1_000,
        );
        // In-window failures: clear regression vs the 0.0 baseline.
        for _ in 0..10 {
            q.record_quality_gate(false);
        }
        let transitions = q.evaluate_observations(2_000);
        assert_eq!(transitions, vec![("p1".into(), ProposalState::Reverted)]);
        // Prior value restored...
        assert!(
            (q.document()
                .feedback_loops
                .immediate
                .quality_gate
                .threshold_default
                - 0.7)
                .abs()
                < 1e-9
        );
        // ...and the revert is auditable: apply + revert trace entries.
        assert_eq!(q.trace.len(), 2);
        let p = q.proposals().into_iter().find(|p| p.id == "p1").unwrap();
        assert!(p.disposition_reason.unwrap().contains("regression"));
    }

    #[test]
    fn revert_removes_set_insertions() {
        let mut q = queue();
        q.ingest(
            proposal(
                "p1",
                TighteningEffect::AddQuarantine {
                    scope: QuarantineScope::Tool,
                    entity: "game-rl:reset".into(),
                },
                BlastRadius::SingleEntity,
            ),
            1_000,
        );
        assert!(q.quarantined.contains("Tool:game-rl:reset"));
        for _ in 0..5 {
            q.record_quality_gate(false);
        }
        q.evaluate_observations(2_000);
        assert!(!q.quarantined.contains("Tool:game-rl:reset"));
    }

    #[test]
    fn duplicate_deny_is_rejected_as_no_op() {
        let mut q = queue();
        q.ingest(
            proposal(
                "p1",
                TighteningEffect::DenyTool { tool: "x".into() },
                BlastRadius::SingleEntity,
            ),
            0,
        );
        let state = q.ingest(
            proposal(
                "p2",
                TighteningEffect::DenyTool { tool: "x".into() },
                BlastRadius::SingleEntity,
            ),
            0,
        );
        assert_eq!(state, ProposalState::Rejected);
    }

    #[test]
    fn no_policy_rejects_everything() {
        let mut doc = arkavo_arp::parse(DOC).unwrap();
        doc.proposal_policy = None;
        let trace = Arc::new(DecisionTrace::new(DecisionTraceConfig {
            enabled: Some(true),
            storage: Some("append_only".into()),
            retention_days: None,
            cryptographic_signing: None,
        }));
        let mut q = ProposalQueue::new(doc, trace, "test".into());
        let state = q.ingest(
            proposal(
                "p1",
                TighteningEffect::DenyTool { tool: "x".into() },
                BlastRadius::SingleEntity,
            ),
            0,
        );
        assert_eq!(state, ProposalState::Rejected);
    }

    #[test]
    fn review_unknown_or_already_decided_errors() {
        let mut q = queue();
        assert!(q.review("missing", true, "op", 0).is_err());
        q.ingest(
            proposal(
                "p1",
                TighteningEffect::DenyTool { tool: "x".into() },
                BlastRadius::SingleEntity,
            ),
            0,
        );
        // Auto-applied — no longer awaiting review.
        assert!(q.review("p1", true, "op", 0).is_err());
    }

    #[test]
    fn layer_budget_introduction_and_revert_clears_it() {
        let mut q = queue();
        q.ingest(
            proposal(
                "p1",
                TighteningEffect::LowerLayerBudget {
                    layer: BudgetLayerName::Execution,
                    new_limit_usd: 0.5,
                },
                BlastRadius::SingleEntity,
            ),
            1_000,
        );
        assert_eq!(q.layer_limit(BudgetLayerName::Execution), Some(0.5));
        for _ in 0..5 {
            q.record_quality_gate(false);
        }
        q.evaluate_observations(2_000);
        // No limit existed before; revert clears it.
        assert_eq!(q.layer_limit(BudgetLayerName::Execution), None);
    }
}
