/// Sequence divergence score with classification
#[derive(Debug)]
pub struct DivergenceResult {
    pub score: f64,
    pub action: DivergenceAction,
}

#[derive(Debug, PartialEq)]
pub enum DivergenceAction {
    Allow,
    AsyncAlert,
    SynchronousGate,
}

pub struct SequenceAnomalyDetector {
    _threshold_low: f64,
    _threshold_high: f64,
}

impl SequenceAnomalyDetector {
    pub fn new(threshold_low: f64, threshold_high: f64) -> Self {
        Self {
            _threshold_low: threshold_low,
            _threshold_high: threshold_high,
        }
    }

    /// SEQ-006: Compute graph edit distance to nearest baseline
    pub fn evaluate(
        &self,
        _current_graph: &[ActionNode],
        _baseline: &[ActionNode],
    ) -> DivergenceResult {
        DivergenceResult {
            score: 0.5,
            action: DivergenceAction::Allow,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActionNode {
    pub tool_name: String,
    pub params_hash: u64,
    pub taint_labels: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline_graph() -> Vec<ActionNode> {
        vec![
            ActionNode {
                tool_name: "read_file".into(),
                params_hash: 123,
                taint_labels: vec![],
            },
            ActionNode {
                tool_name: "summarize".into(),
                params_hash: 456,
                taint_labels: vec!["internal".into()],
            },
        ]
    }

    // =========================================================================
    // SEQ-006: Detect sequence divergence from baseline
    // =========================================================================

    #[test]
    fn low_divergence_allows_action() {
        let detector = SequenceAnomalyDetector::new(0.3, 0.7);
        let current = baseline_graph();
        let baseline = baseline_graph();
        let result = detector.evaluate(&current, &baseline);
        assert_eq!(result.action, DivergenceAction::Allow);
        assert!(result.score < 0.3);
    }

    #[test]
    fn medium_divergence_triggers_async_alert() {
        let detector = SequenceAnomalyDetector::new(0.3, 0.7);
        let mut current = baseline_graph();
        current.push(ActionNode {
            tool_name: "unexpected_tool".into(),
            params_hash: 789,
            taint_labels: vec![],
        });
        let baseline = baseline_graph();
        let result = detector.evaluate(&current, &baseline);
        assert_eq!(result.action, DivergenceAction::AsyncAlert);
    }

    #[test]
    fn high_divergence_triggers_synchronous_gate() {
        let detector = SequenceAnomalyDetector::new(0.3, 0.7);
        let current = vec![
            ActionNode {
                tool_name: "read_credentials".into(),
                params_hash: 999,
                taint_labels: vec!["credentials".into()],
            },
            ActionNode {
                tool_name: "http_post".into(),
                params_hash: 888,
                taint_labels: vec!["credentials".into()],
            },
        ];
        let baseline = baseline_graph();
        let result = detector.evaluate(&current, &baseline);
        assert_eq!(result.action, DivergenceAction::SynchronousGate);
        assert!(result.score > 0.7);
    }

    #[test]
    fn identical_graph_has_zero_divergence() {
        let detector = SequenceAnomalyDetector::new(0.3, 0.7);
        let graph = baseline_graph();
        let result = detector.evaluate(&graph, &graph);
        assert!((result.score - 0.0).abs() < f64::EPSILON);
    }
}
