//! State storage, observability, and DecisionTrace types (§16-§17).

use serde::{Deserialize, Serialize};

use crate::model::{BetaPrior, Sensitivity};

// --- State Storage (§16) ---

/// Learned state storage configuration (§16).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStorage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_cache: Option<StorageBackend>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prior_distributions: Option<PriorStorage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axiom_store: Option<AxiomStorage>,
}

/// Storage backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageBackend {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<StorageType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity: Option<IntegrityMethod>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_at_rest: Option<bool>,
}

/// Available storage backend types.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageType {
    IrohContentAddressed,
    LocalEncrypted,
    ExternalKv,
}

/// Integrity verification method.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityMethod {
    HashChain,
    MerkleVerified,
    None,
}

/// Prior distribution storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorStorage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_interval_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_signed_by: Option<String>,
}

/// Axiom store storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxiomStorage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_consolidation_signature: Option<bool>,
}

// --- Observability (§17) ---

/// Observability and auditability configuration (§17).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_trace: Option<DecisionTraceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export: Option<Export>,
}

/// DecisionTrace audit log configuration (§17.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionTraceConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cryptographic_signing: Option<CryptoSigningConfig>,
}

/// Cryptographic signing configuration for trace entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoSigningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing_required_above_sensitivity: Option<Sensitivity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed_by: Option<String>,
}

/// Export format configurations (§17.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Export {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub siem_export: Option<ExportConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grc_export: Option<ExportConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regulatory_export: Option<ExportConfig>,
}

/// Configuration for a single export destination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<ExportFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<ExportFrequency>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<serde_json::Value>,
}

/// Export output format.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    JsonLines,
    Csv,
    OtelTraces,
    Custom,
}

/// Export frequency.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportFrequency {
    Realtime,
    Hourly,
    Daily,
}

// --- DecisionTrace Entry (§17.1.1) ---

/// A single DecisionTrace entry for the audit log (§17.1.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionTraceEntry {
    pub trace_id: String,
    pub timestamp: String,
    pub task_id: String,
    pub agent_id: String,
    pub layer: TraceLayer,
    pub event_type: TraceEventType,
    pub decision: TraceDecision,
    pub outcome: TraceOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<TraceBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalation: Option<TraceEscalation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback: Option<TraceFeedback>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cryptographic_proofs: Option<TraceCryptoProofs>,
}

/// Layer that produced the trace entry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceLayer {
    Cognitive,
    Execution,
    DataSovereignty,
    Network,
}

/// Type of event recorded in the trace.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceEventType {
    RoutingDecision,
    QualityGate,
    ToolInvocation,
    DataAccess,
    Delegation,
    Escalation,
    BudgetEvent,
    Quarantine,
    HitlAction,
}

/// Decision details: who chose what and why.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceDecision {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chosen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternatives_considered: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_method: Option<SelectionMethod>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prior_state: Option<BetaPrior>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posterior_state: Option<BetaPrior>,
}

/// Method used to select the chosen entity.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMethod {
    ThompsonSampling,
    PolicyCache,
    StaticRule,
    HitlOverride,
}

/// Outcome details: what happened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceOutcome {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
}

/// Budget state at the time of the trace entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceBudget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_task_budget_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spend_this_event_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_spend_cumulative_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub velocity_spend_this_minute_usd: Option<f64>,
}

/// Escalation context in a trace entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEscalation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triggered: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_layer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_layers: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_taken: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

/// Feedback details: how the system learned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceFeedback {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prior_updated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_cache_written: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gossip_propagated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lesson_key: Option<String>,
}

/// Cryptographic proofs attached to a trace entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceCryptoProofs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vc_presented: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_approval_signature: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_storage_backends() {
        let json = r#"{
            "policy_cache": {
                "backend": "iroh_content_addressed",
                "integrity": "hash_chain",
                "encryption_at_rest": true
            }
        }"#;
        let storage: StateStorage = serde_json::from_str(json).unwrap();
        let cache = storage.policy_cache.unwrap();
        assert_eq!(cache.backend, Some(StorageType::IrohContentAddressed));
        assert_eq!(cache.integrity, Some(IntegrityMethod::HashChain));
    }

    #[test]
    fn decision_trace_entry() {
        let json = r#"{
            "trace_id": "550e8400-e29b-41d4-a716-446655440000",
            "timestamp": "2026-03-17T12:00:00Z",
            "task_id": "660e8400-e29b-41d4-a716-446655440001",
            "agent_id": "did:web:example.com:agent:1",
            "layer": "execution",
            "event_type": "tool_invocation",
            "decision": {
                "chosen": "shell_exec",
                "selection_method": "thompson_sampling",
                "prior_state": {"alpha": 5.0, "beta": 2.0},
                "posterior_state": {"alpha": 6.0, "beta": 2.0}
            },
            "outcome": {
                "success": true,
                "quality_score": 0.85,
                "latency_ms": 150,
                "cost_usd": 0.003
            }
        }"#;
        let entry: DecisionTraceEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.layer, TraceLayer::Execution);
        assert_eq!(entry.event_type, TraceEventType::ToolInvocation);
        assert_eq!(entry.outcome.success, Some(true));
        let decision = &entry.decision;
        assert_eq!(decision.chosen.as_deref(), Some("shell_exec"));
        let prior = decision.prior_state.unwrap();
        assert_eq!(prior.alpha, 5.0);
    }

    #[test]
    fn observability_config() {
        let json = r#"{
            "decision_trace": {
                "enabled": true,
                "storage": "append_only",
                "retention_days": 365,
                "cryptographic_signing": {
                    "signing_required_above_sensitivity": "confidential",
                    "algorithm": "Ed25519",
                    "signed_by": "did:web:example.com:agent:1"
                }
            },
            "export": {
                "siem_export": {
                    "format": "json_lines",
                    "endpoint": "https://siem.example.com/ingest",
                    "frequency": "realtime"
                }
            }
        }"#;
        let obs: Observability = serde_json::from_str(json).unwrap();
        let trace = obs.decision_trace.unwrap();
        assert_eq!(trace.enabled, Some(true));
        let signing = trace.cryptographic_signing.unwrap();
        assert_eq!(
            signing.signing_required_above_sensitivity,
            Some(Sensitivity::Confidential)
        );
    }
}
