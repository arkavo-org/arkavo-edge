use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SequenceEvidence {
    pub session_id: Uuid,
    pub action_graph: Vec<AuditActionNode>,
    pub taint_chain: Vec<String>,
    pub baseline_comparison: Option<BaselineComparison>,
    pub correlation_id: Option<Uuid>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditActionNode {
    pub tool_name: String,
    pub sequence_number: u64,
    pub taint_labels: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BaselineComparison {
    pub expected_pattern: String,
    pub actual_pattern: String,
    pub divergence_score: f64,
}

impl SequenceEvidence {
    pub fn from_violation(
        session_id: Uuid,
        action_graph: Vec<AuditActionNode>,
        taint_chain: Vec<String>,
    ) -> Self {
        Self {
            session_id,
            action_graph,
            taint_chain,
            baseline_comparison: None,
            correlation_id: None,
        }
    }

    pub fn with_baseline(mut self, comparison: BaselineComparison) -> Self {
        self.baseline_comparison = Some(comparison);
        self
    }

    pub fn with_correlation(mut self, correlation_id: Uuid) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    pub fn to_event_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "session_id": self.session_id.to_string(),
            "action_graph": self.action_graph,
            "taint_chain": self.taint_chain,
            "baseline_comparison": self.baseline_comparison,
            "correlation_id": self.correlation_id.map(|id| id.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

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

    #[spec("SEQ-015")]
    #[test]
    fn evidence_preserves_full_action_graph() {
        let evidence =
            SequenceEvidence::from_violation(Uuid::new_v4(), sample_graph(), sample_taint_chain());
        assert_eq!(evidence.action_graph.len(), 2);
    }

    #[spec("SEQ-015")]
    #[test]
    fn evidence_preserves_taint_chain_from_source_to_violation() {
        let evidence =
            SequenceEvidence::from_violation(Uuid::new_v4(), sample_graph(), sample_taint_chain());
        assert_eq!(evidence.taint_chain.len(), 3);
        assert!(evidence.taint_chain[0].contains("source"));
        assert!(evidence.taint_chain[2].contains("violation"));
    }

    #[spec("SEQ-015")]
    #[test]
    fn evidence_attaches_baseline_comparison() {
        let evidence =
            SequenceEvidence::from_violation(Uuid::new_v4(), sample_graph(), sample_taint_chain())
                .with_baseline(BaselineComparison {
                    expected_pattern: "read\u{2192}summarize\u{2192}respond".into(),
                    actual_pattern: "read\u{2192}http_post".into(),
                    divergence_score: 0.85,
                });
        assert!(evidence.baseline_comparison.is_some());
    }

    #[spec("SEQ-015")]
    #[test]
    fn evidence_links_correlation_id_for_decomposition() {
        let correlation = Uuid::new_v4();
        let evidence =
            SequenceEvidence::from_violation(Uuid::new_v4(), sample_graph(), sample_taint_chain())
                .with_correlation(correlation);
        assert_eq!(evidence.correlation_id, Some(correlation));
    }

    #[spec("SEQ-015")]
    #[test]
    fn evidence_serializes_to_json_with_required_fields() {
        let evidence =
            SequenceEvidence::from_violation(Uuid::new_v4(), sample_graph(), sample_taint_chain());
        let payload = evidence.to_event_payload();
        assert!(payload.is_object());
        assert!(payload.get("action_graph").is_some());
        assert!(payload.get("taint_chain").is_some());
    }
}
