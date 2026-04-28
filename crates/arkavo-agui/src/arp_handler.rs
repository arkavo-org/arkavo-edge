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
use arkavo_observability::decision_trace::DecisionTrace;
use arkavo_policy_cache::PolicyCache;
use tokio::sync::RwLock;

use crate::types::{
    AgentArpStatus, ArpAdaptationSnapshot, ArpBudgetSummary, ArpDecisionTraceEntry,
    ArpDocumentSummary, ArpPolicyCacheConfigSummary, ArpPolicyCacheEntry, ArpPolicyCacheSnapshot,
    ArpQualityGateSummary, ArpStatusSnapshot,
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
    policy_cache: Option<Arc<PolicyCache>>,
    decision_trace: Option<Arc<DecisionTrace>>,
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
        let mut guard = self.agents.write().await;
        guard.entry(agent_id.to_string()).or_default().doc_state = Some(loaded);
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
        let mut guard = self.agents.write().await;
        guard.entry(agent_id.to_string()).or_default().doc_state = Some(loaded);
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

    /// Build the mesh-wide snapshot pushed to the UI.
    pub async fn snapshot(&self) -> ArpStatusSnapshot {
        let guard = self.agents.read().await;
        let mut agents: Vec<AgentArpStatus> = guard
            .iter()
            .map(|(id, e)| build_agent_status(id, e))
            .collect();
        // Stable ordering: (local) first, then alphabetical
        agents.sort_by(|a, b| match (a.agent_id.as_str(), b.agent_id.as_str()) {
            (LOCAL_AGENT_ID, LOCAL_AGENT_ID) => std::cmp::Ordering::Equal,
            (LOCAL_AGENT_ID, _) => std::cmp::Ordering::Less,
            (_, LOCAL_AGENT_ID) => std::cmp::Ordering::Greater,
            (a, b) => a.cmp(b),
        });

        ArpStatusSnapshot {
            agents,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }
}

fn build_agent_status(agent_id: &str, entry: &AgentEntry) -> AgentArpStatus {
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

    let policy_cache = entry
        .policy_cache
        .as_ref()
        .map(|cache| ArpPolicyCacheSnapshot {
            entry_count: cache.len(),
            chain_valid: cache.verify_chain(),
            entries: collect_known_keys(cache),
        });

    let adaptation: Option<ArpAdaptationSnapshot> = None;

    let decision_traces = entry
        .decision_trace
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
        decision_traces,
    }
}

/// PolicyCache is currently opaque externally; expose only counts until
/// the cache crate ships an iteration API.
fn collect_known_keys(_cache: &PolicyCache) -> Vec<ArpPolicyCacheEntry> {
    Vec::new()
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
}
