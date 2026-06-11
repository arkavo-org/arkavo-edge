//! Agent Runtime Policy (ARP) handler for AG-UI.
//!
//! Each agent in the mesh owns its own ARP, so this handler is keyed by
//! agent_id. The gateway includes its locally-loaded document (via
//! `ARKAVO_ARP_PATH` env var or `./arkavo.arp.json`) under the synthetic
//! `(local)` agent_id until per-agent A2A polling is wired.
//!
//! Runtime PolicyCache and DecisionTrace attachment is also per-agent —
//! callers attach an instance for a specific agent_id, and the snapshot
//! includes only that agent's runtime state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arkavo_arp::ArpDocument;
use arkavo_arp::adaptation::AdaptationMethod;
use arkavo_arp::feedback::DecayStrategy;
use arkavo_arp::observability::{SelectionMethod, TraceEventType, TraceLayer};
use arkavo_arp_runtime::ArpRuntime;
use arkavo_observability::decision_trace::DecisionTrace;
use arkavo_policy_cache::{PolicyCache, PolicySource};
use tokio::sync::RwLock;

use crate::types::{
    AgentArpStatus, ArpAdaptationEntity, ArpAdaptationSnapshot, ArpBudgetSummary,
    ArpDecisionTraceEntry, ArpDocumentSummary, ArpPolicyCacheConfigSummary, ArpPolicyCacheEntry,
    ArpPolicyCacheSnapshot, ArpProposalSnapshot, ArpQualityGateSummary, ArpStatusSnapshot,
    ArpTraceRef, FlightContext,
};

const DEFAULT_DOC_FILENAME: &str = "arkavo.arp.json";
const TRACE_SNAPSHOT_LIMIT: usize = 50;

/// Synthetic agent_id used for the gateway-loaded document, until each
/// real agent reports its own ARP via A2A.
pub const LOCAL_AGENT_ID: &str = "(local)";

/// Tracks ARP state for each agent in the mesh.
pub struct ArpHandler {
    agents: RwLock<HashMap<String, AgentEntry>>,
}

#[derive(Default)]
struct AgentEntry {
    doc_state: Option<LoadedDocument>,
    /// ARP runtime instantiated from the loaded document. Owns the
    /// PolicyCache and AdaptationEngine that the conductor evaluates on
    /// every agent step.
    runtime: Option<Arc<ArpRuntime>>,
    /// Optional explicit cache override (e.g. when the conductor lives in
    /// a separate process and hands its cache to the gateway).
    policy_cache: Option<Arc<PolicyCache>>,
    decision_trace: Option<Arc<DecisionTrace>>,
    /// Set when this entry represents a role within a registered SwarmFlight.
    /// Surfaced in the snapshot so the UI can render kit/role metadata
    /// alongside the per-role runtime state.
    flight_context: Option<FlightContext>,
}

struct LoadedDocument {
    path: Option<PathBuf>,
    parsed: Option<ArpDocument>,
    error: Option<String>,
}

impl Default for ArpHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl ArpHandler {
    pub fn new() -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
        }
    }

    /// Locate the gateway's ARP document and load it as the `(local)` agent.
    /// Tries `ARKAVO_ARP_PATH` env var, then `./arkavo.arp.json`.
    pub async fn load_from_environment(&self) {
        let path = std::env::var("ARKAVO_ARP_PATH")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                let default = Path::new(DEFAULT_DOC_FILENAME);
                if default.is_file() {
                    Some(default.to_path_buf())
                } else {
                    None
                }
            });

        if let Some(path) = path {
            self.load_from_path(LOCAL_AGENT_ID, &path).await;
        }
    }

    /// Load an agent's ARP document from a path.
    pub async fn load_from_path(&self, agent_id: &str, path: &Path) {
        let loaded = match std::fs::read_to_string(path) {
            Ok(json) => match arkavo_arp::parse(&json) {
                Ok(doc) => LoadedDocument {
                    path: Some(path.to_path_buf()),
                    parsed: Some(doc),
                    error: None,
                },
                Err(e) => LoadedDocument {
                    path: Some(path.to_path_buf()),
                    parsed: None,
                    error: Some(e.to_string()),
                },
            },
            Err(e) => LoadedDocument {
                path: Some(path.to_path_buf()),
                parsed: None,
                error: Some(format!("read error: {e}")),
            },
        };
        self.install_loaded(agent_id, loaded).await;
    }

    /// Set an agent's ARP document directly from a JSON string (e.g. pushed
    /// via A2A by that agent).
    pub async fn set_agent_arp(&self, agent_id: &str, json: &str) {
        let loaded = match arkavo_arp::parse(json) {
            Ok(doc) => LoadedDocument {
                path: None,
                parsed: Some(doc),
                error: None,
            },
            Err(e) => LoadedDocument {
                path: None,
                parsed: None,
                error: Some(e.to_string()),
            },
        };
        self.install_loaded(agent_id, loaded).await;
    }

    /// Apply a findings-inbox HITL decision to one agent's proposal queue.
    /// Returns the resulting state label, or an error when the agent has no
    /// runtime or the proposal is not awaiting review.
    pub async fn review_proposal(
        &self,
        agent_id: &str,
        proposal_id: &str,
        approve: bool,
        reviewer: &str,
    ) -> Result<String, String> {
        let runtime = {
            let guard = self.agents.read().await;
            guard
                .get(agent_id)
                .and_then(|e| e.runtime.clone())
                .ok_or_else(|| format!("agent {agent_id} has no ARP runtime"))?
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let queue = runtime.proposal_queue();
        let mut guard = queue.lock().await;
        let state = guard.review(proposal_id, approve, reviewer, now)?;
        Ok(format!("{state:?}").to_lowercase())
    }

    /// Stash a parsed/erroneous document and, when valid, instantiate the
    /// ArpRuntime so the panel shows live cache and adaptation state.
    async fn install_loaded(&self, agent_id: &str, loaded: LoadedDocument) {
        let runtime = loaded.parsed.as_ref().map(|doc| {
            // Reuse the process-global runtime when its document matches —
            // otherwise the panel would show an empty cache while the
            // conductor was writing into a different instance.
            if agent_id == LOCAL_AGENT_ID
                && let Some(global) = arkavo_arp_runtime::current()
            {
                return global;
            }
            Arc::new(ArpRuntime::from_document(doc))
        });

        // For the (local) agent, install the runtime as the process global so
        // the conductor's tool loop can find it. set() is best-effort: if
        // a runtime is already installed (e.g. the gateway started before
        // a sibling subsystem), keep it.
        if let Some(rt) = runtime.as_ref()
            && agent_id == LOCAL_AGENT_ID
        {
            let _ = arkavo_arp_runtime::install(rt.clone());
        }

        let mut guard = self.agents.write().await;
        let entry = guard.entry(agent_id.to_string()).or_default();
        entry.doc_state = Some(loaded);
        entry.runtime = runtime;
    }

    /// Attach a PolicyCache for a specific agent.
    pub async fn attach_policy_cache(&self, agent_id: &str, cache: Arc<PolicyCache>) {
        let mut guard = self.agents.write().await;
        guard.entry(agent_id.to_string()).or_default().policy_cache = Some(cache);
    }

    /// Attach a DecisionTrace for a specific agent.
    pub async fn attach_decision_trace(&self, agent_id: &str, trace: Arc<DecisionTrace>) {
        let mut guard = self.agents.write().await;
        guard
            .entry(agent_id.to_string())
            .or_default()
            .decision_trace = Some(trace);
    }

    /// Forget all state for an agent (e.g. on disconnect).
    pub async fn remove_agent(&self, agent_id: &str) {
        let mut guard = self.agents.write().await;
        guard.remove(agent_id);
    }

    /// Register one role of a SwarmFlight as an agent in the panel.
    ///
    /// SwarmKit's spec §1.2 / §5 hands off from `agent_provisioning` to ARP
    /// at SwarmFlight start. This method makes the per-role ARP state visible
    /// in the same panel that mesh agents publish to. The runtime is reused
    /// — no rebuild — so the cache/adaptation/trace shown in the panel are
    /// the same instances the SwarmFlight is mutating.
    ///
    /// The synthetic agent_id is `flight:{flight_id}:{role_id}` so multiple
    /// concurrent flights with overlapping role_ids do not collide. The UI
    /// uses `FlightContext` to render the friendly kit_name + role_type.
    pub async fn attach_flight_role(&self, registration: FlightRoleRegistration) {
        let FlightRoleRegistration {
            flight_id,
            kit_id,
            kit_name,
            role_id,
            role_type,
            arp_doc,
            runtime,
            bound_agent_did,
        } = registration;
        let agent_id = flight_role_agent_id(flight_id, &role_id);
        let flight_context = FlightContext {
            flight_id: flight_id.to_string(),
            kit_id,
            kit_name,
            role_id,
            role_type,
            bound_agent_did,
        };
        let loaded = LoadedDocument {
            path: None,
            parsed: Some(arp_doc),
            error: None,
        };
        let mut guard = self.agents.write().await;
        let entry = guard.entry(agent_id).or_default();
        entry.doc_state = Some(loaded);
        entry.runtime = Some(runtime);
        entry.flight_context = Some(flight_context);
    }

    /// Remove every role registered under `flight_id`. Called when a
    /// SwarmFlight terminates so stale entries don't accumulate.
    pub async fn remove_flight(&self, flight_id: uuid::Uuid) {
        let prefix = format!("flight:{flight_id}:");
        let mut guard = self.agents.write().await;
        guard.retain(|k, _| !k.starts_with(&prefix));
    }

    /// Build the mesh-wide snapshot pushed to the UI.
    pub async fn snapshot(&self) -> ArpStatusSnapshot {
        let guard = self.agents.read().await;
        let mut agents: Vec<AgentArpStatus> = Vec::with_capacity(guard.len());
        for (id, e) in guard.iter() {
            agents.push(build_agent_status(id, e).await);
        }
        // Stable ordering: (local) first, then mesh agents alphabetically,
        // then flight roles grouped by flight_id then role_id. This keeps
        // the dropdown sections visually consistent.
        agents.sort_by(sort_status);

        ArpStatusSnapshot {
            agents,
            timestamp: chrono::Utc::now().to_rfc3339(),
            swarmkit_launch_errors: Vec::new(),
        }
    }
}

fn sort_status(a: &AgentArpStatus, b: &AgentArpStatus) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let a_local = a.agent_id == LOCAL_AGENT_ID;
    let b_local = b.agent_id == LOCAL_AGENT_ID;
    if a_local || b_local {
        return match (a_local, b_local) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => Ordering::Equal,
        };
    }
    // Flight roles always sort after mesh agents.
    match (&a.flight_context, &b.flight_context) {
        (None, None) => a.agent_id.cmp(&b.agent_id),
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(fa), Some(fb)) => fa
            .flight_id
            .cmp(&fb.flight_id)
            .then_with(|| fa.role_id.cmp(&fb.role_id)),
    }
}

/// Synthetic agent_id used to address one role of a SwarmFlight in the
/// per-agent ARP map. Exposed for use in tests and bridge code.
pub fn flight_role_agent_id(flight_id: uuid::Uuid, role_id: &str) -> String {
    format!("flight:{flight_id}:{role_id}")
}

/// All inputs required to surface one SwarmFlight role in the ARP panel.
/// Bundled so `ArpHandler::attach_flight_role` stays ergonomic while the
/// data the UI needs to render the role grows.
pub struct FlightRoleRegistration {
    pub flight_id: uuid::Uuid,
    pub kit_id: String,
    pub kit_name: String,
    pub role_id: String,
    pub role_type: String,
    pub arp_doc: ArpDocument,
    pub runtime: Arc<ArpRuntime>,
    /// DID of the mesh agent the orchestrator bound to this role.
    /// `None` for in-process / unbound flights.
    pub bound_agent_did: Option<String>,
}

async fn build_agent_status(agent_id: &str, entry: &AgentEntry) -> AgentArpStatus {
    let (path_str, valid, error, summary) = match &entry.doc_state {
        None => (None, false, None, None),
        Some(loaded) => {
            let path_str = loaded.path.as_ref().map(|p| p.display().to_string());
            match (&loaded.parsed, &loaded.error) {
                (Some(doc), _) => (path_str, true, None, Some(summarize_document(doc))),
                (None, Some(err)) => (path_str, false, Some(err.clone()), None),
                (None, None) => (path_str, false, None, None),
            }
        }
    };

    // Prefer an explicitly attached cache; otherwise read from the runtime
    // instantiated from the loaded ARP document.
    let cache_arc = entry
        .policy_cache
        .clone()
        .or_else(|| entry.runtime.as_ref().map(|rt| rt.cache()));

    let policy_cache = cache_arc.as_ref().map(|cache| ArpPolicyCacheSnapshot {
        entry_count: cache.len(),
        chain_valid: cache.verify_chain(),
        entries: cache
            .snapshot()
            .into_iter()
            .map(|e| ArpPolicyCacheEntry {
                key: e.key,
                source: policy_source_label(e.source).to_string(),
                influence: e.influence,
                age_sec: e.age_sec,
            })
            .collect(),
    });

    let adaptation: Option<ArpAdaptationSnapshot> = match entry.runtime.as_ref() {
        Some(rt) => {
            let eng = rt.adaptation();
            let guard = eng.lock().await;
            Some(ArpAdaptationSnapshot {
                method: adaptation_method_label(guard.method()).to_string(),
                entities: guard
                    .snapshot()
                    .into_iter()
                    .map(|p| ArpAdaptationEntity {
                        id: p.id,
                        alpha: p.alpha,
                        beta: p.beta,
                        mean: p.mean,
                        observations: p.observations,
                        in_warmup: p.in_warmup,
                        live_alpha: p.live_alpha,
                        live_beta: p.live_beta,
                        distilled_alpha: p.distilled_alpha,
                        distilled_beta: p.distilled_beta,
                        distilled_weight: p.distilled_weight,
                    })
                    .collect(),
            })
        }
        None => None,
    };

    let proposals = match entry.runtime.as_ref() {
        Some(rt) => {
            let queue = rt.proposal_queue();
            let guard = queue.lock().await;
            guard
                .proposals()
                .into_iter()
                .map(convert_proposal)
                .collect()
        }
        None => Vec::new(),
    };

    // Prefer an explicitly attached trace; otherwise pull from the
    // runtime (which now emits an entry per tool outcome).
    let trace_arc = entry
        .decision_trace
        .clone()
        .or_else(|| entry.runtime.as_ref().map(|rt| rt.decision_trace()));

    let decision_traces = trace_arc
        .as_ref()
        .map(|trace| {
            let mut entries: Vec<_> = trace
                .snapshot()
                .into_iter()
                .map(convert_trace_entry)
                .collect();
            if entries.len() > TRACE_SNAPSHOT_LIMIT {
                let drop_n = entries.len() - TRACE_SNAPSHOT_LIMIT;
                entries.drain(0..drop_n);
            }
            entries
        })
        .unwrap_or_default();

    AgentArpStatus {
        agent_id: agent_id.to_string(),
        document_path: path_str,
        document_valid: valid,
        load_error: error,
        document: summary,
        policy_cache,
        adaptation,
        proposals,
        decision_traces,
        flight_context: entry.flight_context.clone(),
    }
}

/// Render a TighteningProposal as a findings-inbox card payload.
fn convert_proposal(p: arkavo_arp::proposal::TighteningProposal) -> ArpProposalSnapshot {
    use arkavo_arp::proposal::TighteningEffect;
    let (effect_kind, effect_summary) = match &p.effect {
        TighteningEffect::LowerTaskCeiling { new_ceiling_usd } => (
            "lower_task_ceiling",
            format!("lower task ceiling to ${new_ceiling_usd}"),
        ),
        TighteningEffect::LowerLayerBudget {
            layer,
            new_limit_usd,
        } => (
            "lower_layer_budget",
            format!("lower {layer:?} layer budget to ${new_limit_usd}"),
        ),
        TighteningEffect::RaiseQualityGateThreshold { new_threshold } => (
            "raise_quality_gate_threshold",
            format!("raise quality gate to {new_threshold}"),
        ),
        TighteningEffect::AddQuarantine { scope, entity } => {
            ("add_quarantine", format!("quarantine {entity} ({scope:?})"))
        }
        TighteningEffect::DenyTool { tool } => ("deny_tool", format!("deny tool {tool}")),
        TighteningEffect::DenyEgress { host } => ("deny_egress", format!("deny egress to {host}")),
        TighteningEffect::LowerRateLimit {
            new_requests_per_second,
        } => (
            "lower_rate_limit",
            format!("lower rate limit to {new_requests_per_second}/s"),
        ),
    };
    ArpProposalSnapshot {
        id: p.id,
        origin: format!("{:?}", p.origin).to_lowercase(),
        state: format!("{:?}", p.state).to_lowercase(),
        effect_kind: effect_kind.to_string(),
        effect_summary,
        rationale: p.rationale,
        blast_radius: blast_radius_label(p.blast_radius).to_string(),
        evidence: p
            .evidence
            .into_iter()
            .map(|t| ArpTraceRef {
                trace_id: t.trace_id,
                episode_ids: t.episode_ids,
            })
            .collect(),
        created_at: p.created_at,
        applied_at: p.applied_at,
        reviewed_by: p.reviewed_by,
        disposition_reason: p.disposition_reason,
    }
}

fn blast_radius_label(r: arkavo_arp::proposal::BlastRadius) -> &'static str {
    use arkavo_arp::proposal::BlastRadius;
    match r {
        BlastRadius::SingleEntity => "single_entity",
        BlastRadius::SingleAgent => "single_agent",
        BlastRadius::Flight => "flight",
        BlastRadius::Mesh => "mesh",
    }
}

fn policy_source_label(source: PolicySource) -> &'static str {
    match source {
        PolicySource::Automated => "automated",
        PolicySource::Human => "human",
        PolicySource::Incident => "incident",
    }
}

fn summarize_document(doc: &ArpDocument) -> ArpDocumentSummary {
    let mut sections = vec![
        "adaptation".to_string(),
        "feedback_loops".to_string(),
        "budget".to_string(),
    ];
    if doc.cognitive.is_some() {
        sections.push("cognitive".to_string());
    }
    if doc.execution.is_some() {
        sections.push("execution".to_string());
    }
    if doc.data_sovereignty.is_some() {
        sections.push("data_sovereignty".to_string());
    }
    if doc.network.is_some() {
        sections.push("network".to_string());
    }
    if doc.escalation.is_some() {
        sections.push("escalation".to_string());
    }
    if doc.quarantine.is_some() {
        sections.push("quarantine".to_string());
    }
    if doc.hitl.is_some() {
        sections.push("hitl".to_string());
    }
    if doc.session.is_some() {
        sections.push("session".to_string());
    }
    if doc.state_storage.is_some() {
        sections.push("state_storage".to_string());
    }
    if doc.observability.is_some() {
        sections.push("observability".to_string());
    }

    let imm = &doc.feedback_loops.immediate;
    let quality_gate = Some(ArpQualityGateSummary {
        threshold: imm.quality_gate.threshold_default,
        metric: serde_label(&imm.quality_gate.metric),
        on_failure: serde_label(&imm.quality_gate.on_failure),
    });

    let st = &doc.feedback_loops.short_term;
    let policy_cache_config = Some(ArpPolicyCacheConfigSummary {
        ttl_sec: st.policy_cache.default_ttl_sec,
        decay_strategy: decay_strategy_label(st.policy_cache.decay_strategy).to_string(),
        half_life_sec: st.policy_cache.decay_half_life_sec,
        human_exempt: st
            .policy_cache
            .human_source_exempt_from_decay
            .unwrap_or(true),
    });

    ArpDocumentSummary {
        arp_spec: doc.arp_spec.clone(),
        adl_uri: doc.adl_ref.uri.clone(),
        adl_hash: doc.adl_ref.document_hash.clone(),
        integrity_signed: doc.integrity.is_some(),
        adaptation_method: adaptation_method_label(doc.adaptation.method).to_string(),
        quality_gate,
        policy_cache_config,
        budget: ArpBudgetSummary {
            task_ceiling_usd: doc.budget.task_ceiling_usd,
            on_exhaustion: serde_label(&doc.budget.on_exhaustion),
            max_spend_per_minute_usd: Some(doc.budget.velocity.max_spend_per_minute_usd),
            max_tool_calls_per_minute: doc.budget.velocity.max_tool_calls_per_minute,
        },
        sections_present: sections,
    }
}

/// Serialize a serde-tagged enum to its `snake_case` string form
/// (matching the values used in ARP JSON documents).
fn serde_label<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

fn adaptation_method_label(method: AdaptationMethod) -> &'static str {
    match method {
        AdaptationMethod::ThompsonSampling => "thompson_sampling",
        AdaptationMethod::EpsilonGreedy => "epsilon_greedy",
        AdaptationMethod::Ucb1 => "ucb1",
        AdaptationMethod::Static => "static",
    }
}

fn decay_strategy_label(strategy: DecayStrategy) -> &'static str {
    match strategy {
        DecayStrategy::Exponential => "exponential",
        DecayStrategy::Linear => "linear",
        DecayStrategy::Step => "step",
        DecayStrategy::None => "none",
    }
}

fn trace_layer_label(layer: TraceLayer) -> &'static str {
    match layer {
        TraceLayer::Cognitive => "cognitive",
        TraceLayer::Execution => "execution",
        TraceLayer::DataSovereignty => "data_sovereignty",
        TraceLayer::Network => "network",
    }
}

fn trace_event_label(event: TraceEventType) -> &'static str {
    match event {
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

fn selection_method_label(method: SelectionMethod) -> &'static str {
    match method {
        SelectionMethod::ThompsonSampling => "thompson_sampling",
        SelectionMethod::PolicyCache => "policy_cache",
        SelectionMethod::StaticRule => "static_rule",
        SelectionMethod::HitlOverride => "hitl_override",
    }
}

fn convert_trace_entry(
    entry: arkavo_arp::observability::DecisionTraceEntry,
) -> ArpDecisionTraceEntry {
    ArpDecisionTraceEntry {
        trace_id: entry.trace_id,
        layer: trace_layer_label(entry.layer).to_string(),
        event_type: trace_event_label(entry.event_type).to_string(),
        agent_id: entry.agent_id,
        task_id: entry.task_id,
        chosen_entity: entry.decision.chosen,
        selection_method: entry
            .decision
            .selection_method
            .map(|m| selection_method_label(m).to_string()),
        success: entry.outcome.success,
        denied_reason: entry.outcome.error_type,
        timestamp: entry.timestamp,
    }
}

#[cfg(test)]
mod tests {
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

    #[tokio::test]
    async fn snapshot_with_no_agents() {
        let handler = ArpHandler::new();
        let snap = handler.snapshot().await;
        assert!(snap.agents.is_empty());
    }

    #[tokio::test]
    async fn snapshot_after_loading_local_doc() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.arp.json");
        std::fs::write(&path, MIN_DOC).unwrap();

        let handler = ArpHandler::new();
        handler.load_from_path(LOCAL_AGENT_ID, &path).await;

        let snap = handler.snapshot().await;
        assert_eq!(snap.agents.len(), 1);
        let local = &snap.agents[0];
        assert_eq!(local.agent_id, LOCAL_AGENT_ID);
        assert!(local.document_valid);
        let doc = local.document.as_ref().expect("summary present");
        assert_eq!(doc.arp_spec, "0.1.0");
        assert_eq!(doc.adaptation_method, "thompson_sampling");
    }

    #[tokio::test]
    async fn snapshot_with_invalid_doc() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.arp.json");
        std::fs::write(&path, "{not valid json").unwrap();

        let handler = ArpHandler::new();
        handler.load_from_path(LOCAL_AGENT_ID, &path).await;

        let snap = handler.snapshot().await;
        assert_eq!(snap.agents.len(), 1);
        assert!(!snap.agents[0].document_valid);
        assert!(snap.agents[0].load_error.is_some());
    }

    #[tokio::test]
    async fn multi_agent_snapshot_orders_local_first() {
        let handler = ArpHandler::new();
        handler.set_agent_arp("rover-beta", MIN_DOC).await;
        handler.set_agent_arp("rover-alpha", MIN_DOC).await;
        handler.set_agent_arp(LOCAL_AGENT_ID, MIN_DOC).await;

        let snap = handler.snapshot().await;
        assert_eq!(snap.agents.len(), 3);
        assert_eq!(snap.agents[0].agent_id, LOCAL_AGENT_ID);
        assert_eq!(snap.agents[1].agent_id, "rover-alpha");
        assert_eq!(snap.agents[2].agent_id, "rover-beta");
    }

    #[tokio::test]
    async fn remove_agent_drops_state() {
        let handler = ArpHandler::new();
        handler.set_agent_arp("rover-beta", MIN_DOC).await;
        handler.remove_agent("rover-beta").await;
        let snap = handler.snapshot().await;
        assert!(snap.agents.is_empty());
    }

    #[tokio::test]
    async fn loaded_doc_exposes_live_runtime_state() {
        // After loading a valid ARP document, the ArpHandler should
        // instantiate an ArpRuntime so the panel shows live cache and
        // adaptation state instead of "not yet wired" empty placeholders.
        let handler = ArpHandler::new();
        handler.set_agent_arp("rover-alpha", MIN_DOC).await;

        let snap = handler.snapshot().await;
        let agent = &snap.agents[0];

        let cache = agent.policy_cache.as_ref().expect("policy cache live");
        assert_eq!(cache.entry_count, 0, "cache starts empty but is live");
        assert!(cache.chain_valid, "fresh chain should verify");

        let adaptation = agent.adaptation.as_ref().expect("adaptation engine live");
        assert_eq!(adaptation.method, "thompson_sampling");
        assert!(adaptation.entities.is_empty(), "no priors observed yet");
    }

    fn flight_reg(
        flight_id: uuid::Uuid,
        kit_name: &str,
        role_id: &str,
        role_type: &str,
        runtime: Arc<ArpRuntime>,
    ) -> FlightRoleRegistration {
        FlightRoleRegistration {
            flight_id,
            kit_id: format!("blake3:{kit_name}"),
            kit_name: kit_name.into(),
            role_id: role_id.into(),
            role_type: role_type.into(),
            arp_doc: arkavo_arp::parse(MIN_DOC).unwrap(),
            runtime,
            bound_agent_did: None,
        }
    }

    #[spec("SK-020")]
    #[spec("SK-024")]
    #[tokio::test]
    async fn flight_role_appears_with_context() {
        let handler = ArpHandler::new();
        let doc = arkavo_arp::parse(MIN_DOC).unwrap();
        let runtime = Arc::new(ArpRuntime::from_document(&doc));
        let flight_id = uuid::Uuid::new_v4();

        handler
            .attach_flight_role(flight_reg(
                flight_id,
                "Tiny Kit",
                "worker",
                "specialist",
                runtime.clone(),
            ))
            .await;

        let snap = handler.snapshot().await;
        assert_eq!(snap.agents.len(), 1);
        let entry = &snap.agents[0];
        assert_eq!(
            entry.agent_id,
            format!("flight:{flight_id}:worker"),
            "synthetic agent_id namespaces by flight_id"
        );
        let ctx = entry.flight_context.as_ref().expect("flight context set");
        assert_eq!(ctx.kit_name, "Tiny Kit");
        assert_eq!(ctx.role_id, "worker");
        assert_eq!(ctx.role_type, "specialist");
        assert_eq!(ctx.flight_id, flight_id.to_string());

        // Document panel populated from the supplied ARP doc.
        assert!(entry.document_valid);
        let doc_summary = entry.document.as_ref().unwrap();
        assert_eq!(doc_summary.adaptation_method, "thompson_sampling");

        // Runtime panels populated from the runtime — empty so far.
        let cache = entry.policy_cache.as_ref().unwrap();
        assert_eq!(cache.entry_count, 0);
        let adaptation = entry.adaptation.as_ref().unwrap();
        assert!(adaptation.entities.is_empty());
    }

    #[spec("SK-020")]
    #[tokio::test]
    async fn flight_role_runtime_state_flows_through_snapshot() {
        let handler = ArpHandler::new();
        let doc = arkavo_arp::parse(MIN_DOC).unwrap();
        let runtime = Arc::new(ArpRuntime::from_document(&doc));
        let flight_id = uuid::Uuid::new_v4();

        handler
            .attach_flight_role(flight_reg(
                flight_id,
                "Demo Kit",
                "analyst",
                "asset_analyst",
                runtime.clone(),
            ))
            .await;

        // Drive the same runtime instance the handler is observing.
        runtime
            .record_tool_outcome("asset.summarize", true, 0.91)
            .await;

        let snap = handler.snapshot().await;
        let entry = &snap.agents[0];
        assert_eq!(entry.decision_traces.len(), 1);
        assert_eq!(
            entry.decision_traces[0].chosen_entity.as_deref(),
            Some("asset.summarize")
        );
        assert_eq!(entry.policy_cache.as_ref().unwrap().entry_count, 1);
    }

    #[spec("SK-021")]
    #[tokio::test]
    async fn remove_flight_drops_only_its_roles() {
        let handler = ArpHandler::new();
        let doc = arkavo_arp::parse(MIN_DOC).unwrap();
        let f1 = uuid::Uuid::new_v4();
        let f2 = uuid::Uuid::new_v4();

        for (flight, role) in [(f1, "alpha"), (f1, "beta"), (f2, "alpha")] {
            handler
                .attach_flight_role(flight_reg(
                    flight,
                    "Kit",
                    role,
                    "specialist",
                    Arc::new(ArpRuntime::from_document(&doc)),
                ))
                .await;
        }
        handler.set_agent_arp("rover-alpha", MIN_DOC).await;

        assert_eq!(handler.snapshot().await.agents.len(), 4);
        handler.remove_flight(f1).await;

        let snap = handler.snapshot().await;
        assert_eq!(snap.agents.len(), 2);
        assert!(
            snap.agents
                .iter()
                .any(|a| a.agent_id == format!("flight:{f2}:alpha"))
        );
        assert!(snap.agents.iter().any(|a| a.agent_id == "rover-alpha"));
    }

    #[spec("SK-023")]
    #[tokio::test]
    async fn snapshot_sort_order_local_then_mesh_then_flight() {
        let handler = ArpHandler::new();
        handler.set_agent_arp("rover-zulu", MIN_DOC).await;
        handler.set_agent_arp("rover-alpha", MIN_DOC).await;
        handler.set_agent_arp(LOCAL_AGENT_ID, MIN_DOC).await;
        let doc = arkavo_arp::parse(MIN_DOC).unwrap();
        let flight_id = uuid::Uuid::new_v4();
        handler
            .attach_flight_role(flight_reg(
                flight_id,
                "Kit",
                "worker",
                "specialist",
                Arc::new(ArpRuntime::from_document(&doc)),
            ))
            .await;

        let snap = handler.snapshot().await;
        assert_eq!(snap.agents[0].agent_id, LOCAL_AGENT_ID);
        assert_eq!(snap.agents[1].agent_id, "rover-alpha");
        assert_eq!(snap.agents[2].agent_id, "rover-zulu");
        assert_eq!(
            snap.agents[3].agent_id,
            format!("flight:{flight_id}:worker")
        );
    }

    #[tokio::test]
    async fn tool_outcome_surfaces_in_decision_trace_snapshot() {
        // The runtime's DecisionTrace must flow through to the gateway
        // snapshot so the panel's Recent Decision Traces table populates
        // as the conductor records tool outcomes.
        let handler = ArpHandler::new();
        handler.set_agent_arp("rover-alpha", MIN_DOC).await;

        // Reach into the runtime owned by the handler and record an
        // outcome — same path the conductor uses, just direct.
        let rt = {
            let guard = handler.agents.read().await;
            guard
                .get("rover-alpha")
                .and_then(|e| e.runtime.clone())
                .expect("runtime instantiated")
        };
        rt.record_tool_outcome("filesystem_tools", true, 0.92).await;

        let snap = handler.snapshot().await;
        let agent = &snap.agents[0];
        assert_eq!(agent.decision_traces.len(), 1);
        let entry = &agent.decision_traces[0];
        assert_eq!(entry.event_type, "tool_invocation");
        assert_eq!(entry.layer, "execution");
        assert_eq!(entry.chosen_entity.as_deref(), Some("filesystem_tools"));
        assert_eq!(entry.success, Some(true));
    }
}

#[cfg(test)]
mod proposal_tests {
    // #[tokio::test] expands to Runtime::block_on; harmless in tests.
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use arkavo_arp::proposal::{
        BlastRadius, ProposalOrigin, ProposalState, TighteningEffect, TighteningProposal, TraceRef,
    };
    use arkavo_test_macros::spec;

    const DOC_WITH_POLICY: &str = r#"{
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

    fn proposal(id: &str, radius: BlastRadius) -> TighteningProposal {
        TighteningProposal {
            id: id.into(),
            origin: ProposalOrigin::Consolidation,
            state: ProposalState::Proposed,
            effect: TighteningEffect::DenyTool {
                tool: "game-rl:reset".into(),
            },
            rationale: "reset correlated with colony loss".into(),
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

    async fn ingest_into(handler: &ArpHandler, agent_id: &str, p: TighteningProposal) {
        let runtime = {
            let guard = handler.agents.read().await;
            guard.get(agent_id).and_then(|e| e.runtime.clone()).unwrap()
        };
        let queue = runtime.proposal_queue();
        queue.lock().await.ingest(p, 1_000);
    }

    #[spec("AGUI-013")]
    #[tokio::test]
    async fn proposals_surface_in_snapshot_as_inbox_cards() {
        let handler = ArpHandler::new();
        handler.set_agent_arp("rover-alpha", DOC_WITH_POLICY).await;
        ingest_into(
            &handler,
            "rover-alpha",
            proposal("p1", BlastRadius::SingleEntity),
        )
        .await;

        let snapshot = handler.snapshot().await;
        let agent = snapshot
            .agents
            .iter()
            .find(|a| a.agent_id == "rover-alpha")
            .unwrap();
        assert_eq!(agent.proposals.len(), 1);
        let card = &agent.proposals[0];
        assert_eq!(card.effect_kind, "deny_tool");
        // Within the auto-apply radius: applied and observing.
        assert_eq!(card.state, "observed");
        assert_eq!(card.evidence[0].episode_ids, vec!["ep-1".to_string()]);
    }

    #[spec("AGUI-014")]
    #[tokio::test]
    async fn review_proposal_approves_held_proposal() {
        let handler = ArpHandler::new();
        handler.set_agent_arp("rover-alpha", DOC_WITH_POLICY).await;
        // single_agent radius exceeds the single_entity auto-apply ceiling.
        ingest_into(
            &handler,
            "rover-alpha",
            proposal("p2", BlastRadius::SingleAgent),
        )
        .await;

        let state = handler
            .review_proposal("rover-alpha", "p2", true, "operator")
            .await
            .unwrap();
        assert_eq!(state, "observed");

        let snapshot = handler.snapshot().await;
        let agent = snapshot
            .agents
            .iter()
            .find(|a| a.agent_id == "rover-alpha")
            .unwrap();
        assert_eq!(agent.proposals[0].reviewed_by.as_deref(), Some("operator"));
    }

    #[tokio::test]
    async fn review_proposal_errors_for_unknown_agent() {
        let handler = ArpHandler::new();
        let err = handler
            .review_proposal("ghost", "p1", true, "operator")
            .await
            .unwrap_err();
        assert!(err.contains("no ARP runtime"));
    }
}
