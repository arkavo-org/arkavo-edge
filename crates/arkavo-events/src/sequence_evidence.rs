use uuid::Uuid;

/// Serialized action graph for audit trail
#[derive(Debug, Clone)]
pub struct SequenceEvidence {
    pub session_id: Uuid,
    pub action_graph: Vec<AuditActionNode>,
    pub taint_chain: Vec<String>,
    pub baseline_comparison: Option<BaselineComparison>,
    pub correlation_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct AuditActionNode {
    pub tool_name: String,
    pub sequence_number: u64,
    pub taint_labels: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BaselineComparison {
    pub expected_pattern: String,
    pub actual_pattern: String,
    pub divergence_score: f64,
}

impl SequenceEvidence {
    /// SEQ-015: Create evidence from a sequence violation
    pub fn from_violation(
        _session_id: Uuid,
        _action_graph: Vec<AuditActionNode>,
        _taint_chain: Vec<String>,
    ) -> Self {
        Self {
            session_id: Uuid::nil(),
            action_graph: Vec::new(),
            taint_chain: Vec::new(),
            baseline_comparison: None,
            correlation_id: None,
        }
    }

    /// Attach baseline comparison for forensic context
    pub fn with_baseline(self, _comparison: BaselineComparison) -> Self {
        self
    }

    /// Link to cross-session decomposition analysis
    pub fn with_correlation(self, _correlation_id: Uuid) -> Self {
        self
    }

    /// Serialize for inclusion in Event payload
    pub fn to_event_payload(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_graph() -> Vec<AuditActionNode> {
        vec![
            AuditActionNode {
                tool_name: "read_file".into(),
                sequence_number: 1,
                taint_labels: vec!["internal".into()],
            },
            AuditActionNode {
                tool_name: "http_post".into(),
                sequence_number: 2,
                taint_labels: vec!["internal".into()],
            },
        ]
    }

    fn sample_taint_chain() -> Vec<String> {
        vec![
            "source:intranet".into(),
            "transform:summarize".into(),
            "violation:egress_blocked".into(),
        ]
    }

    // =========================================================================
    // SEQ-015: Sequence evidence in audit events
    // =========================================================================

    #[test]
    fn evidence_includes_full_action_graph() {
        let evidence = SequenceEvidence::from_violation(
            Uuid::new_v4(),
            sample_graph(),
            sample_taint_chain(),
        );
        assert_eq!(evidence.action_graph.len(), 2);
    }

    #[test]
    fn evidence_includes_taint_chain() {
        let evidence = SequenceEvidence::from_violation(
            Uuid::new_v4(),
            sample_graph(),
            sample_taint_chain(),
        );
        assert_eq!(evidence.taint_chain.len(), 3);
        assert!(evidence.taint_chain[0].contains("source"));
        assert!(evidence.taint_chain[2].contains("violation"));
    }

    #[test]
    fn evidence_includes_baseline_comparison() {
        let evidence = SequenceEvidence::from_violation(
            Uuid::new_v4(),
            sample_graph(),
            sample_taint_chain(),
        )
        .with_baseline(BaselineComparison {
            expected_pattern: "read→summarize→respond".into(),
            actual_pattern: "read→http_post".into(),
            divergence_score: 0.85,
        });
        assert!(evidence.baseline_comparison.is_some());
        let comparison = evidence.baseline_comparison.unwrap();
        assert!(comparison.divergence_score > 0.5);
    }

    #[test]
    fn evidence_links_correlation_id_for_decomposition() {
        let correlation = Uuid::new_v4();
        let evidence = SequenceEvidence::from_violation(
            Uuid::new_v4(),
            sample_graph(),
            sample_taint_chain(),
        )
        .with_correlation(correlation);
        assert_eq!(evidence.correlation_id, Some(correlation));
    }

    #[test]
    fn evidence_serializes_to_json_payload() {
        let evidence = SequenceEvidence::from_violation(
            Uuid::new_v4(),
            sample_graph(),
            sample_taint_chain(),
        );
        let payload = evidence.to_event_payload();
        assert!(payload.is_object());
        assert!(payload.get("action_graph").is_some());
        assert!(payload.get("taint_chain").is_some());
    }
}
