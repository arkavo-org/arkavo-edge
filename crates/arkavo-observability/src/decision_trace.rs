//! Append-only DecisionTrace audit log per ARP §17.1.
//!
//! Records all policy enforcement events as immutable, optionally-signed
//! trace entries. Entries are stored in insertion order and can be exported
//! as JSON lines for SIEM/GRC integration.

use std::sync::{Arc, Mutex};

use arkavo_arp::observability::{
    DecisionTraceConfig, DecisionTraceEntry, TraceDecision, TraceEventType, TraceLayer,
    TraceOutcome,
};
use chrono::Utc;
use serde_json;
use uuid::Uuid;

/// Append-only DecisionTrace audit log.
///
/// Thread-safe via `Arc<Mutex<Vec<_>>>`. Entries cannot be modified or
/// removed after insertion — only appended and read.
pub struct DecisionTrace {
    entries: Arc<Mutex<Vec<StoredEntry>>>,
    config: DecisionTraceConfig,
    #[cfg(feature = "signing")]
    signing_key: Option<arkavo_crypto::AgentKeypair>,
}

/// An entry stored in the trace with optional signature.
#[derive(Debug, Clone)]
struct StoredEntry {
    entry: DecisionTraceEntry,
    signature: Option<String>,
}

impl DecisionTrace {
    /// Create a new DecisionTrace from ARP observability config (§17.1).
    pub fn new(config: DecisionTraceConfig) -> Self {
        Self {
            entries: Arc::new(Mutex::new(Vec::new())),
            config,
            #[cfg(feature = "signing")]
            signing_key: None,
        }
    }

    /// Set the Ed25519 signing key for entries above the configured sensitivity.
    #[cfg(feature = "signing")]
    pub fn with_signing_key(mut self, key: arkavo_crypto::AgentKeypair) -> Self {
        self.signing_key = Some(key);
        self
    }

    /// Whether the trace is enabled per config.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled.unwrap_or(true)
    }

    /// Record a new trace entry. Returns the trace_id assigned.
    ///
    /// If signing is enabled and the entry's sensitivity warrants it,
    /// the entry is signed with the configured Ed25519 key.
    pub fn record(
        &self,
        layer: TraceLayer,
        event_type: TraceEventType,
        task_id: Uuid,
        agent_id: &str,
        decision: TraceDecision,
        outcome: TraceOutcome,
    ) -> Option<String> {
        if !self.is_enabled() {
            return None;
        }

        let trace_id = Uuid::new_v4().to_string();
        let entry = DecisionTraceEntry {
            trace_id: trace_id.clone(),
            timestamp: Utc::now().to_rfc3339(),
            task_id: task_id.to_string(),
            agent_id: agent_id.to_string(),
            layer,
            event_type,
            decision,
            outcome,
            budget: None,
            escalation: None,
            feedback: None,
            cryptographic_proofs: None,
        };

        let signature = self.maybe_sign(&entry);

        tracing::debug!(
            trace_id = %trace_id,
            layer = ?layer,
            event_type = ?event_type,
            "DecisionTrace entry recorded"
        );

        let stored = StoredEntry { entry, signature };
        self.entries.lock().unwrap().push(stored);

        Some(trace_id)
    }

    /// Record a full pre-built entry (for cases where caller sets all fields).
    pub fn record_entry(&self, entry: DecisionTraceEntry) -> Option<String> {
        if !self.is_enabled() {
            return None;
        }

        let trace_id = entry.trace_id.clone();
        let signature = self.maybe_sign(&entry);
        let stored = StoredEntry { entry, signature };
        self.entries.lock().unwrap().push(stored);

        Some(trace_id)
    }

    /// Number of recorded entries.
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    /// Whether the trace is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Export all entries as JSON lines (one JSON object per line).
    pub fn export_json_lines(&self) -> String {
        let entries = self.entries.lock().unwrap();
        entries
            .iter()
            .filter_map(|stored| serde_json::to_string(&stored.entry).ok())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Get a snapshot of all entries (cloned).
    pub fn snapshot(&self) -> Vec<DecisionTraceEntry> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.entry.clone())
            .collect()
    }

    /// Get the signature for an entry by trace_id, if it was signed.
    pub fn get_signature(&self, trace_id: &str) -> Option<String> {
        let entries = self.entries.lock().unwrap();
        entries
            .iter()
            .find(|s| s.entry.trace_id == trace_id)
            .and_then(|s| s.signature.clone())
    }

    /// Sign an entry if signing is configured and sensitivity warrants it.
    fn maybe_sign(&self, _entry: &DecisionTraceEntry) -> Option<String> {
        #[cfg(feature = "signing")]
        {
            use arkavo_arp::model::Sensitivity;

            let threshold = self
                .config
                .cryptographic_signing
                .as_ref()
                .and_then(|c| c.signing_required_above_sensitivity)
                .unwrap_or(Sensitivity::Confidential);

            if self.signing_key.is_some() && threshold <= Sensitivity::Confidential {
                let json = serde_json::to_vec(_entry).ok()?;
                let sig = self.signing_key.as_ref().unwrap().sign(&json);
                return Some(base64_encode(&sig));
            }
        }
        None
    }
}

#[cfg(feature = "signing")]
fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(data.len() * 4 / 3 + 4);
    // Simple hex encoding (Ed25519 signatures are 64 bytes = 128 hex chars)
    for byte in data {
        write!(s, "{byte:02x}").unwrap();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_arp::model::BetaPrior;
    use arkavo_arp::observability::SelectionMethod;

    fn test_config() -> DecisionTraceConfig {
        DecisionTraceConfig {
            enabled: Some(true),
            storage: Some("append_only".into()),
            retention_days: Some(365),
            cryptographic_signing: None,
        }
    }

    fn test_decision() -> TraceDecision {
        TraceDecision {
            chosen: Some("model_a".into()),
            alternatives_considered: Some(vec!["model_b".into(), "model_c".into()]),
            selection_method: Some(SelectionMethod::ThompsonSampling),
            prior_state: Some(BetaPrior {
                alpha: 5.0,
                beta: 2.0,
            }),
            posterior_state: Some(BetaPrior {
                alpha: 6.0,
                beta: 2.0,
            }),
        }
    }

    fn test_outcome() -> TraceOutcome {
        TraceOutcome {
            success: Some(true),
            quality_score: Some(0.85),
            latency_ms: Some(150),
            cost_usd: Some(0.003),
            error_type: None,
        }
    }

    #[test]
    fn record_and_retrieve() {
        let trace = DecisionTrace::new(test_config());
        let task_id = Uuid::new_v4();

        let trace_id = trace.record(
            TraceLayer::Execution,
            TraceEventType::ToolInvocation,
            task_id,
            "did:web:example.com:agent:1",
            test_decision(),
            test_outcome(),
        );

        assert!(trace_id.is_some());
        assert_eq!(trace.len(), 1);

        let entries = trace.snapshot();
        assert_eq!(entries[0].layer, TraceLayer::Execution);
        assert_eq!(entries[0].event_type, TraceEventType::ToolInvocation);
        assert_eq!(entries[0].decision.chosen.as_deref(), Some("model_a"));
    }

    #[test]
    fn disabled_trace_records_nothing() {
        let config = DecisionTraceConfig {
            enabled: Some(false),
            storage: None,
            retention_days: None,
            cryptographic_signing: None,
        };
        let trace = DecisionTrace::new(config);

        let result = trace.record(
            TraceLayer::Cognitive,
            TraceEventType::QualityGate,
            Uuid::new_v4(),
            "agent-1",
            test_decision(),
            test_outcome(),
        );

        assert!(result.is_none());
        assert!(trace.is_empty());
    }

    #[test]
    fn append_only_ordering() {
        let trace = DecisionTrace::new(test_config());

        for i in 0..5 {
            trace.record(
                TraceLayer::Execution,
                TraceEventType::RoutingDecision,
                Uuid::new_v4(),
                &format!("agent-{i}"),
                test_decision(),
                test_outcome(),
            );
        }

        assert_eq!(trace.len(), 5);
        let entries = trace.snapshot();
        for (i, entry) in entries.iter().enumerate() {
            assert_eq!(entry.agent_id, format!("agent-{i}"));
        }
    }

    #[test]
    fn export_json_lines() {
        let trace = DecisionTrace::new(test_config());

        trace.record(
            TraceLayer::Cognitive,
            TraceEventType::QualityGate,
            Uuid::new_v4(),
            "agent-1",
            test_decision(),
            test_outcome(),
        );
        trace.record(
            TraceLayer::Network,
            TraceEventType::Delegation,
            Uuid::new_v4(),
            "agent-2",
            test_decision(),
            test_outcome(),
        );

        let export = trace.export_json_lines();
        let lines: Vec<&str> = export.lines().collect();
        assert_eq!(lines.len(), 2);

        // Each line should be valid JSON
        for line in lines {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(parsed.get("trace_id").is_some());
            assert!(parsed.get("layer").is_some());
        }
    }

    #[test]
    fn record_full_entry() {
        let trace = DecisionTrace::new(test_config());

        let entry = DecisionTraceEntry {
            trace_id: "custom-id-123".into(),
            timestamp: Utc::now().to_rfc3339(),
            task_id: Uuid::new_v4().to_string(),
            agent_id: "did:web:example.com:agent:custom".into(),
            layer: TraceLayer::DataSovereignty,
            event_type: TraceEventType::DataAccess,
            decision: test_decision(),
            outcome: test_outcome(),
            budget: None,
            escalation: None,
            feedback: None,
            cryptographic_proofs: None,
        };

        let id = trace.record_entry(entry);
        assert_eq!(id.as_deref(), Some("custom-id-123"));
        assert_eq!(trace.len(), 1);
    }

    #[test]
    fn all_event_types_recordable() {
        let trace = DecisionTrace::new(test_config());
        let event_types = [
            TraceEventType::RoutingDecision,
            TraceEventType::QualityGate,
            TraceEventType::ToolInvocation,
            TraceEventType::DataAccess,
            TraceEventType::Delegation,
            TraceEventType::Escalation,
            TraceEventType::BudgetEvent,
            TraceEventType::Quarantine,
            TraceEventType::HitlAction,
        ];

        for event_type in event_types {
            trace.record(
                TraceLayer::Execution,
                event_type,
                Uuid::new_v4(),
                "agent",
                test_decision(),
                test_outcome(),
            );
        }

        assert_eq!(trace.len(), 9);
    }

    #[test]
    fn unsigned_entries_have_no_signature() {
        let trace = DecisionTrace::new(test_config());
        let trace_id = trace
            .record(
                TraceLayer::Execution,
                TraceEventType::ToolInvocation,
                Uuid::new_v4(),
                "agent",
                test_decision(),
                test_outcome(),
            )
            .unwrap();

        assert!(trace.get_signature(&trace_id).is_none());
    }

    #[test]
    fn empty_trace() {
        let trace = DecisionTrace::new(test_config());
        assert!(trace.is_empty());
        assert_eq!(trace.len(), 0);
        assert!(trace.export_json_lines().is_empty());
        assert!(trace.snapshot().is_empty());
    }
}
