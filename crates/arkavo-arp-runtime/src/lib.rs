//! Per-agent ARP runtime: bundles `PolicyCache`, `AdaptationEngine`, and
//! the append-only `DecisionTrace` so the conductor can update all three
//! as a single unit on every agent step.
//!
//! `ArpRuntime` is constructed from a parsed `ArpDocument` and exposes:
//!
//! * `record_tool_outcome` — called after each tool invocation in the
//!   conductor loop. Updates the AdaptationEngine prior for the tool,
//!   writes a quality-tagged entry into the PolicyCache, and appends a
//!   `tool_invocation` entry to the DecisionTrace with prior/posterior
//!   state. This is the audit trail required by ARP §17.1.
//! * `cache()` / `adaptation()` / `decision_trace()` — accessors for the
//!   gateway and UI to snapshot live state.
//!
//! A process-global accessor (`install` / `current`) lets the conductor
//! reach the runtime without threading an extra argument through every
//! tool-loop call site. The CLI installs an instance once at startup; the
//! standalone AG-UI gateway constructs its own when no agent is running.

pub mod proposal_queue;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use arkavo_adaptation::AdaptationEngine;
use arkavo_arp::ArpDocument;
use arkavo_arp::model::BetaPrior;
use arkavo_arp::observability::{
    DecisionTraceConfig, SelectionMethod, TraceDecision, TraceEventType, TraceLayer, TraceOutcome,
};
use arkavo_arp::proposal::ProposalState;
use arkavo_observability::decision_trace::DecisionTrace;
use arkavo_policy_cache::{PolicyCache, PolicySource};
pub use proposal_queue::ProposalQueue;
use serde_json::json;
use tokio::sync::Mutex;

/// Default agent_id used when the conductor doesn't pass one explicitly.
/// Matches the gateway's synthetic local-agent id.
const DEFAULT_AGENT_ID: &str = "(local)";

/// Optional per-call context attached to a tool outcome. Lets the
/// conductor enrich DecisionTrace entries with task and timing data
/// without making `record_tool_outcome` ergonomically noisy.
#[derive(Debug, Clone, Default)]
pub struct ToolOutcomeContext {
    pub task_id: Option<uuid::Uuid>,
    pub agent_id: Option<String>,
    pub latency_ms: Option<u64>,
    pub error_type: Option<String>,
}

impl ToolOutcomeContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_task_id(mut self, id: uuid::Uuid) -> Self {
        self.task_id = Some(id);
        self
    }

    pub fn with_agent_id<S: Into<String>>(mut self, id: S) -> Self {
        self.agent_id = Some(id.into());
        self
    }

    pub fn with_latency_ms(mut self, ms: u64) -> Self {
        self.latency_ms = Some(ms);
        self
    }

    pub fn with_error_type<S: Into<String>>(mut self, err: S) -> Self {
        self.error_type = Some(err.into());
        self
    }
}

/// Bundles the PolicyCache, AdaptationEngine, and DecisionTrace
/// instantiated from an ARP document. Cheap to clone — backed by `Arc`s.
pub struct ArpRuntime {
    cache: Arc<PolicyCache>,
    adaptation: Arc<Mutex<AdaptationEngine>>,
    decision_trace: Arc<DecisionTrace>,
    quality_threshold: f64,
    /// Tightening-proposal queue. Owns the canonical post-apply document;
    /// an applied RaiseQualityGateThreshold takes effect on the next
    /// outcome recorded through this runtime.
    proposals: Arc<Mutex<ProposalQueue>>,
    /// Monotonic counter so each cache key is unique. Required because the
    /// PolicyCache hash chain doesn't tolerate same-key overwrites.
    outcome_seq: AtomicU64,
}

impl ArpRuntime {
    /// Build a runtime from a parsed ARP document. Quality threshold is
    /// pulled from `feedback_loops.immediate.quality_gate.threshold_default`.
    /// The DecisionTrace is configured from `observability.decision_trace`
    /// when present, otherwise enabled with append-only defaults.
    pub fn from_document(doc: &ArpDocument) -> Self {
        let cache_cfg = doc.feedback_loops.short_term.policy_cache.clone();
        let cache = Arc::new(PolicyCache::new(cache_cfg));
        let adaptation = Arc::new(Mutex::new(AdaptationEngine::new(doc.adaptation.clone())));
        let quality_threshold = doc.feedback_loops.immediate.quality_gate.threshold_default;

        let trace_cfg = doc
            .observability
            .as_ref()
            .and_then(|o| o.decision_trace.clone())
            .unwrap_or(DecisionTraceConfig {
                enabled: Some(true),
                storage: Some("append_only".into()),
                retention_days: None,
                cryptographic_signing: None,
            });
        let decision_trace = Arc::new(DecisionTrace::new(trace_cfg));
        // Publish it so layers below the runtime — the conductor's egress gate
        // among them — can record without the trace threading through them.
        arkavo_observability::decision_trace::install(decision_trace.clone());
        let proposals = Arc::new(Mutex::new(ProposalQueue::new(
            doc.clone(),
            decision_trace.clone(),
            DEFAULT_AGENT_ID.to_string(),
        )));

        Self {
            cache,
            adaptation,
            decision_trace,
            quality_threshold,
            proposals,
            outcome_seq: AtomicU64::new(0),
        }
    }

    /// The tightening-proposal queue for this runtime.
    pub fn proposal_queue(&self) -> Arc<Mutex<ProposalQueue>> {
        self.proposals.clone()
    }

    pub fn cache(&self) -> Arc<PolicyCache> {
        self.cache.clone()
    }

    pub fn adaptation(&self) -> Arc<Mutex<AdaptationEngine>> {
        self.adaptation.clone()
    }

    pub fn decision_trace(&self) -> Arc<DecisionTrace> {
        self.decision_trace.clone()
    }

    /// Advance proposal observation windows and return the transitions made.
    ///
    /// This is exposed so callers can drive evaluation from a periodic tick in
    /// addition to the implicit evaluation that happens on every recorded tool
    /// outcome (see [`record_tool_outcome_with`]).
    pub async fn evaluate_observations_at(
        &self,
        now_epoch_sec: u64,
    ) -> Vec<(String, ProposalState)> {
        let mut queue = self.proposals.lock().await;
        queue.evaluate_observations(now_epoch_sec)
    }

    /// Quality threshold below which an outcome is treated as a failure
    /// when updating priors and the cache.
    pub fn quality_threshold(&self) -> f64 {
        self.quality_threshold
    }

    /// Record a tool's outcome into the AdaptationEngine prior, the
    /// PolicyCache, and the DecisionTrace. Called once per tool invocation
    /// in the conductor.
    ///
    /// `quality` is the response/result quality in [0.0, 1.0]. Successes
    /// above the configured threshold reinforce the prior; below-threshold
    /// outcomes degrade it.
    pub async fn record_tool_outcome(&self, tool_name: &str, success: bool, quality: f64) {
        self.record_tool_outcome_with(tool_name, success, quality, &ToolOutcomeContext::default())
            .await;
    }

    /// Same as `record_tool_outcome` but accepts a `ToolOutcomeContext`
    /// with task_id, latency, and error type that get propagated into
    /// the DecisionTrace entry.
    pub async fn record_tool_outcome_with(
        &self,
        tool_name: &str,
        success: bool,
        quality: f64,
        ctx: &ToolOutcomeContext,
    ) {
        let q = quality.clamp(0.0, 1.0);
        // The proposal queue owns the post-apply document: an applied
        // RaiseQualityGateThreshold takes effect here. The queue also tallies
        // gate outcomes to evaluate observation windows.
        let above_gate = {
            let mut queue = self.proposals.lock().await;
            let threshold = queue
                .document()
                .feedback_loops
                .immediate
                .quality_gate
                .threshold_default;
            let above = q >= threshold;
            queue.record_quality_gate(above);
            // Advance observation windows on every outcome. This wires the
            // propose → applied → observed → confirmed | reverted lifecycle
            // into the conductor's hot path (see #620).
            let now = chrono::Utc::now().timestamp() as u64;
            queue.evaluate_observations(now);
            above
        };
        let effective_success = success && above_gate;

        // Update the prior, capturing before/after for the trace entry.
        let (prior_before, prior_after) = {
            let mut eng = self.adaptation.lock().await;
            let before = eng.get_prior(tool_name);
            eng.update(
                tool_name,
                arkavo_adaptation::Outcome {
                    success: effective_success,
                    quality_score: q,
                },
            );
            let after = eng.get_prior(tool_name);
            (before, after)
        };

        // PolicyCache: append a new entry per outcome.
        let n = self.outcome_seq.fetch_add(1, Ordering::Relaxed);
        let key = format!("tool.outcome.{tool_name}.{n}");
        let value = json!({
            "tool": tool_name,
            "success": effective_success,
            "quality": q,
            "above_quality_gate": above_gate,
        });
        self.cache.insert(key, value, PolicySource::Automated);

        // DecisionTrace: emit a `tool_invocation` entry on the Execution layer.
        let task_id = ctx.task_id.unwrap_or_else(uuid::Uuid::nil);
        let agent_id = ctx.agent_id.as_deref().unwrap_or(DEFAULT_AGENT_ID);
        let error_type = if !effective_success {
            ctx.error_type.clone().or_else(|| {
                Some(
                    if !above_gate {
                        "below_quality_gate"
                    } else {
                        "tool_failure"
                    }
                    .into(),
                )
            })
        } else {
            None
        };

        let decision = TraceDecision {
            chosen: Some(tool_name.to_string()),
            alternatives_considered: None,
            selection_method: Some(SelectionMethod::ThompsonSampling),
            prior_state: Some(BetaPrior {
                alpha: prior_before.alpha,
                beta: prior_before.beta,
            }),
            posterior_state: Some(BetaPrior {
                alpha: prior_after.alpha,
                beta: prior_after.beta,
            }),
        };
        let outcome = TraceOutcome {
            success: Some(effective_success),
            quality_score: Some(q),
            latency_ms: ctx.latency_ms,
            cost_usd: None,
            error_type,
        };
        self.decision_trace.record(
            TraceLayer::Execution,
            TraceEventType::ToolInvocation,
            task_id,
            agent_id,
            decision,
            outcome,
        );
    }
}

/// Process-global ARP runtime — installed by the CLI at startup, looked up
/// by the conductor tool loop and the gateway when no explicit reference
/// is available.
static GLOBAL: OnceLock<Arc<ArpRuntime>> = OnceLock::new();

/// Install the global runtime. Returns `Err` if one is already installed.
pub fn install(rt: Arc<ArpRuntime>) -> Result<(), Arc<ArpRuntime>> {
    GLOBAL.set(rt)
}

/// Current global ARP runtime, if installed.
pub fn current() -> Option<Arc<ArpRuntime>> {
    GLOBAL.get().cloned()
}

// Test helpers — short string forms of the trace enums for assertion-readability.
#[cfg(test)]
trait EnumStringRepr {
    fn to_string_repr(&self) -> &'static str;
}

#[cfg(test)]
impl EnumStringRepr for TraceLayer {
    fn to_string_repr(&self) -> &'static str {
        match self {
            TraceLayer::Cognitive => "cognitive",
            TraceLayer::Execution => "execution",
            TraceLayer::DataSovereignty => "data_sovereignty",
            TraceLayer::Network => "network",
        }
    }
}

#[cfg(test)]
impl EnumStringRepr for TraceEventType {
    fn to_string_repr(&self) -> &'static str {
        match self {
            TraceEventType::RoutingDecision => "routing_decision",
            TraceEventType::QualityGate => "quality_gate",
            TraceEventType::ToolInvocation => "tool_invocation",
            TraceEventType::DataAccess => "data_access",
            TraceEventType::Delegation => "delegation",
            TraceEventType::Escalation => "escalation",
            TraceEventType::BudgetEvent => "budget_event",
            TraceEventType::Quarantine => "quarantine",
            TraceEventType::HitlAction => "hitl_action",
            TraceEventType::ProposalLifecycle => "proposal_lifecycle",
        }
    }
}

#[cfg(test)]
mod tests {
    // #[tokio::test] expands to Runtime::block_on; harmless in tests.
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use arkavo_test_macros::spec;

    const MIN_DOC: &str = r#"{
        "arp_spec": "0.1.0",
        "adl_ref": {"uri": "https://example.com/adl.json", "document_hash": "sha256:abc"},
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
                    "decay_strategy": "exponential",
                    "decay_half_life_sec": 86400
                }
            }
        },
        "budget": {
            "task_ceiling_usd": 2.5,
            "on_exhaustion": "halt_and_report",
            "velocity": {"max_spend_per_minute_usd": 0.5}
        }
    }"#;

    fn make_runtime() -> ArpRuntime {
        let doc = arkavo_arp::parse(MIN_DOC).unwrap();
        ArpRuntime::from_document(&doc)
    }

    #[tokio::test]
    async fn record_success_updates_prior_and_cache() {
        let rt = make_runtime();

        rt.record_tool_outcome("filesystem_tools", true, 0.95).await;

        // Cache now has one entry
        assert_eq!(rt.cache().len(), 1);
        let entries = rt.cache().snapshot();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].key.contains("filesystem_tools"));

        // Adaptation engine has a prior for this tool
        let snap = rt.adaptation().lock().await.snapshot();
        let prior = snap
            .iter()
            .find(|p| p.id == "filesystem_tools")
            .expect("prior exists");
        assert!(prior.alpha > 1.0, "alpha grew on success: {}", prior.alpha);
    }

    #[tokio::test]
    async fn record_below_quality_gate_increments_beta() {
        let rt = make_runtime();

        // Quality 0.4 is below the 0.7 gate; treated as failure.
        rt.record_tool_outcome("shell_exec", true, 0.4).await;

        let snap = rt.adaptation().lock().await.snapshot();
        let prior = snap.iter().find(|p| p.id == "shell_exec").unwrap();
        assert!(
            prior.beta > 1.0,
            "beta grew on below-gate outcome: {}",
            prior.beta
        );
    }

    #[tokio::test]
    async fn cache_chain_remains_valid_after_updates() {
        let rt = make_runtime();
        rt.record_tool_outcome("tool_a", true, 0.9).await;
        rt.record_tool_outcome("tool_b", false, 0.2).await;
        rt.record_tool_outcome("tool_a", true, 0.85).await;
        assert!(rt.cache().verify_chain());
    }

    #[tokio::test]
    async fn record_emits_decision_trace_entry() {
        let rt = make_runtime();
        rt.record_tool_outcome("filesystem_tools", true, 0.95).await;

        let trace = rt.decision_trace();
        assert_eq!(trace.len(), 1);

        let entries = trace.snapshot();
        let entry = &entries[0];
        assert_eq!(entry.event_type.to_string_repr(), "tool_invocation");
        assert_eq!(entry.layer.to_string_repr(), "execution");
        assert_eq!(entry.decision.chosen.as_deref(), Some("filesystem_tools"));
        assert_eq!(entry.outcome.success, Some(true));
        assert!((entry.outcome.quality_score.unwrap() - 0.95).abs() < 1e-9);

        // prior_state and posterior_state both set; alpha grew on success.
        let prior = entry.decision.prior_state.unwrap();
        let posterior = entry.decision.posterior_state.unwrap();
        assert!(posterior.alpha > prior.alpha);
        assert_eq!(posterior.beta, prior.beta);
    }

    #[tokio::test]
    async fn below_gate_outcome_carries_error_type() {
        let rt = make_runtime();
        rt.record_tool_outcome("shell_exec", true, 0.3).await; // below 0.7 gate

        let entries = rt.decision_trace().snapshot();
        let entry = &entries[0];
        assert_eq!(entry.outcome.success, Some(false));
        assert_eq!(
            entry.outcome.error_type.as_deref(),
            Some("below_quality_gate")
        );
    }

    #[tokio::test]
    async fn record_with_context_propagates_task_id_and_latency() {
        let rt = make_runtime();
        let task_id = uuid::Uuid::new_v4();
        let ctx = ToolOutcomeContext::new()
            .with_task_id(task_id)
            .with_agent_id("rover-alpha")
            .with_latency_ms(42);

        rt.record_tool_outcome_with("git_status", true, 0.9, &ctx)
            .await;

        let entries = rt.decision_trace().snapshot();
        let entry = &entries[0];
        assert_eq!(entry.task_id, task_id.to_string());
        assert_eq!(entry.agent_id, "rover-alpha");
        assert_eq!(entry.outcome.latency_ms, Some(42));
    }

    #[tokio::test]
    async fn applied_threshold_raise_bites_on_next_outcome() {
        // An auto-applied RaiseQualityGateThreshold must take effect on the
        // very next recorded outcome — the queue owns the live document.
        use arkavo_arp::proposal::{
            BlastRadius, ProposalOrigin, ProposalState, TighteningEffect, TighteningProposal,
            TraceRef,
        };
        let mut doc = arkavo_arp::parse(MIN_DOC).unwrap();
        doc.proposal_policy = Some(arkavo_arp::proposal::ProposalPolicy {
            accepted_origins: vec![ProposalOrigin::Consolidation],
            auto_apply_max_blast_radius: BlastRadius::SingleEntity,
            review: None,
            observation_window_sec: Some(600),
        });
        let rt = ArpRuntime::from_document(&doc);

        // Quality 0.75 passes the original 0.7 gate.
        rt.record_tool_outcome("tool_x", true, 0.75).await;
        let entries = rt.decision_trace().snapshot();
        assert_eq!(entries.last().unwrap().outcome.success, Some(true));

        let state = rt.proposal_queue().lock().await.ingest(
            TighteningProposal {
                id: "raise-gate".into(),
                origin: ProposalOrigin::Consolidation,
                state: ProposalState::Proposed,
                effect: TighteningEffect::RaiseQualityGateThreshold { new_threshold: 0.8 },
                rationale: "0.7 admitted low-quality outcomes".into(),
                blast_radius: BlastRadius::SingleEntity,
                evidence: vec![TraceRef {
                    trace_id: "t-1".into(),
                    episode_ids: vec![],
                }],
                created_at: "2026-06-10T00:00:00Z".into(),
                applied_at: None,
                reviewed_by: None,
                disposition_reason: None,
            },
            1_000,
        );
        assert_eq!(state, ProposalState::Observed);

        // The same 0.75 quality now fails the raised 0.8 gate.
        rt.record_tool_outcome("tool_x", true, 0.75).await;
        let entries = rt.decision_trace().snapshot();
        let last = entries
            .iter()
            .rfind(|e| e.decision.chosen.as_deref() == Some("tool_x"))
            .unwrap();
        assert_eq!(last.outcome.success, Some(false));
        assert_eq!(
            last.outcome.error_type.as_deref(),
            Some("below_quality_gate")
        );
    }

    #[tokio::test]
    async fn explicit_error_type_overrides_default() {
        let rt = make_runtime();
        let ctx = ToolOutcomeContext::new().with_error_type("connection_refused");
        rt.record_tool_outcome_with("shell_exec", false, 0.0, &ctx)
            .await;

        let entries = rt.decision_trace().snapshot();
        assert_eq!(
            entries[0].outcome.error_type.as_deref(),
            Some("connection_refused")
        );
    }

    #[tokio::test]
    async fn record_tool_outcome_evaluates_observation_windows() {
        use arkavo_arp::proposal::{
            BlastRadius, ProposalOrigin, ProposalState, TighteningEffect, TighteningProposal,
            TraceRef,
        };
        let mut doc = arkavo_arp::parse(MIN_DOC).unwrap();
        doc.proposal_policy = Some(arkavo_arp::proposal::ProposalPolicy {
            accepted_origins: vec![ProposalOrigin::Consolidation],
            auto_apply_max_blast_radius: BlastRadius::SingleEntity,
            review: None,
            // Use a long window so the implicit evaluation during
            // record_tool_outcome_with (real time) does not close it early.
            observation_window_sec: Some(10_000_000_000),
        });
        let rt = ArpRuntime::from_document(&doc);

        let state = rt.proposal_queue().lock().await.ingest(
            TighteningProposal {
                id: "observed-then-confirmed".into(),
                origin: ProposalOrigin::Consolidation,
                state: ProposalState::Proposed,
                effect: TighteningEffect::RaiseQualityGateThreshold { new_threshold: 0.8 },
                rationale: "tighten gate".into(),
                blast_radius: BlastRadius::SingleEntity,
                evidence: vec![TraceRef {
                    trace_id: "t-1".into(),
                    episode_ids: vec![],
                }],
                created_at: "2026-06-10T00:00:00Z".into(),
                applied_at: None,
                reviewed_by: None,
                disposition_reason: None,
            },
            1_000,
        );
        assert_eq!(state, ProposalState::Observed);

        // Record an outcome inside the window; the proposal should stay observed.
        rt.record_tool_outcome_with("tool_x", true, 0.9, &ToolOutcomeContext::default())
            .await;
        let state = rt
            .proposal_queue()
            .lock()
            .await
            .proposal_state("observed-then-confirmed")
            .unwrap();
        assert_eq!(state, ProposalState::Observed);

        // Advance past the window and trigger another evaluation.
        let transitions = rt.evaluate_observations_at(10_000_001_000).await;
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].1, ProposalState::Confirmed);

        let state = rt
            .proposal_queue()
            .lock()
            .await
            .proposal_state("observed-then-confirmed")
            .unwrap();
        assert_eq!(state, ProposalState::Confirmed);
    }

    #[spec("ARP-009")]
    #[tokio::test]
    async fn provenance_tracking_exposes_live_distilled_split() {
        use arkavo_arp::model::BetaPrior;
        let mut doc = arkavo_arp::parse(MIN_DOC).unwrap();
        doc.adaptation.prior_management = Some(arkavo_arp::adaptation::PriorManagement {
            version_binding: None,
            reset_on_version_change: None,
            reset_state: None,
            provenance_tracking: Some(true),
            distilled_decay: Some(arkavo_arp::adaptation::DistilledDecay {
                strategy: arkavo_arp::adaptation::DistilledDecayStrategy::LiveDisplacement,
                displacement_factor: Some(0.05),
                floor: Some(0.0),
            }),
        });
        let rt = ArpRuntime::from_document(&doc);

        // Seed distilled mass for a tool.
        rt.adaptation().lock().await.seed_distilled(
            "tool_x",
            BetaPrior {
                alpha: 4.0,
                beta: 1.0,
            },
        );

        // Record live outcomes.
        rt.record_tool_outcome("tool_x", true, 0.9).await;
        rt.record_tool_outcome("tool_x", true, 0.9).await;

        let snapshot = rt.adaptation().lock().await.snapshot();
        let entry = snapshot.iter().find(|e| e.id == "tool_x").unwrap();
        assert!(entry.live_alpha > 0.0);
        assert!(entry.distilled_alpha.unwrap_or(0.0) > 0.0);
        assert!(entry.distilled_weight.unwrap_or(1.0) < 1.0);
    }

    #[spec("ARP-009")]
    #[tokio::test]
    async fn distilled_floor_preserves_minimum_teacher_mass() {
        use arkavo_arp::model::BetaPrior;
        let mut doc = arkavo_arp::parse(MIN_DOC).unwrap();
        doc.adaptation.prior_management = Some(arkavo_arp::adaptation::PriorManagement {
            version_binding: None,
            reset_on_version_change: None,
            reset_state: None,
            provenance_tracking: Some(true),
            distilled_decay: Some(arkavo_arp::adaptation::DistilledDecay {
                strategy: arkavo_arp::adaptation::DistilledDecayStrategy::LiveDisplacement,
                displacement_factor: Some(0.1),
                floor: Some(0.25),
            }),
        });
        let rt = ArpRuntime::from_document(&doc);

        rt.adaptation().lock().await.seed_distilled(
            "tool_x",
            BetaPrior {
                alpha: 8.0,
                beta: 2.0,
            },
        );

        // With displacement_factor 0.1, ten live observations would fully
        // displace distilled mass. The 0.25 floor must keep a minimum share.
        for _ in 0..10 {
            rt.record_tool_outcome("tool_x", true, 0.9).await;
        }

        let adaptation = rt.adaptation();
        let eng = adaptation.lock().await;
        let snapshot = eng.snapshot();
        let entry = snapshot.iter().find(|e| e.id == "tool_x").unwrap();
        let live = eng.live_prior("tool_x");
        let distilled = eng.distilled_prior("tool_x").unwrap();

        assert!((entry.distilled_weight.unwrap() - 0.25).abs() < 1e-9);
        assert!(
            (entry.alpha - (live.alpha + 0.25 * distilled.alpha)).abs() < 1e-9,
            "effective alpha must combine live and floor-weighted distilled mass"
        );
        assert!(
            (entry.beta - (live.beta + 0.25 * distilled.beta)).abs() < 1e-9,
            "effective beta must combine live and floor-weighted distilled mass"
        );
        assert!(entry.live_alpha > 0.0);
        assert!(entry.distilled_alpha.unwrap() > 0.0);
    }
}
