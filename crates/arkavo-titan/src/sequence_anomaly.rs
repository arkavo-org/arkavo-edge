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
    threshold_low: f64,
    threshold_high: f64,
}

impl SequenceAnomalyDetector {
    pub fn new(threshold_low: f64, threshold_high: f64) -> Self {
        Self {
            threshold_low,
            threshold_high,
        }
    }

    pub fn evaluate(
        &self,
        current_graph: &[ActionNode],
        baseline: &[ActionNode],
    ) -> DivergenceResult {
        let score = Self::graph_edit_distance(current_graph, baseline);
        let action = if score > self.threshold_high {
            DivergenceAction::SynchronousGate
        } else if score > self.threshold_low {
            DivergenceAction::AsyncAlert
        } else {
            DivergenceAction::Allow
        };
        DivergenceResult { score, action }
    }

    fn graph_edit_distance(current: &[ActionNode], baseline: &[ActionNode]) -> f64 {
        if current.is_empty() && baseline.is_empty() {
            return 0.0;
        }

        let max_len = current.len().max(baseline.len());
        let mut matches = 0usize;
        for (i, node) in current.iter().enumerate() {
            if let Some(base_node) = baseline.get(i) {
                if node.tool_name == base_node.tool_name {
                    matches += 1;
                }
            }
        }

        // Factor in taint divergence: new taint labels not in baseline
        let mut taint_diff = 0usize;
        for node in current {
            if !node.taint_labels.is_empty() {
                let has_baseline_match = baseline
                    .iter()
                    .any(|b| b.tool_name == node.tool_name && b.taint_labels == node.taint_labels);
                if !has_baseline_match {
                    taint_diff += 1;
                }
            }
        }

        let len_diff = (current.len() as f64 - baseline.len() as f64).abs();
        let name_diff = max_len.saturating_sub(matches) as f64;
        let taint_penalty = taint_diff as f64 * 0.15;
        let raw = (len_diff + name_diff) / (max_len as f64 + 1.0) + taint_penalty;
        raw.min(1.0)
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
    use arkavo_test_macros::spec;
    use std::time::Instant;

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

    #[spec("SEQ-006")]
    #[test]
    fn identical_graph_produces_zero_divergence() {
        let detector = SequenceAnomalyDetector::new(0.3, 0.7);
        let graph = baseline_graph();
        let result = detector.evaluate(&graph, &graph);
        assert!((result.score - 0.0).abs() < f64::EPSILON);
        assert_eq!(result.action, DivergenceAction::Allow);
    }

    #[spec("SEQ-006")]
    #[test]
    fn low_divergence_allows_action() {
        let detector = SequenceAnomalyDetector::new(0.3, 0.7);
        let current = baseline_graph();
        let baseline = baseline_graph();
        let result = detector.evaluate(&current, &baseline);
        assert!(result.score < 0.3);
    }

    #[spec("SEQ-006")]
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

    #[spec("SEQ-006")]
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

    #[spec("SEQ-006")]
    #[test]
    fn detection_completes_within_latency_budget() {
        let detector = SequenceAnomalyDetector::new(0.3, 0.7);
        let current = baseline_graph();
        let baseline = baseline_graph();
        let start = Instant::now();
        let _result = detector.evaluate(&current, &baseline);
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_micros() < 50,
            "detection took {}μs, budget is 50μs",
            elapsed.as_micros()
        );
    }

    #[spec("SEQ-006")]
    #[test]
    fn novel_workflow_gates_with_human_option_not_hard_block() {
        let detector = SequenceAnomalyDetector::new(0.3, 0.7);
        let novel = vec![
            ActionNode {
                tool_name: "new_read_tool".into(),
                params_hash: 111,
                taint_labels: vec![],
            },
            ActionNode {
                tool_name: "new_summarize_tool".into(),
                params_hash: 222,
                taint_labels: vec!["internal".into()],
            },
        ];
        let baseline = baseline_graph();
        let result = detector.evaluate(&novel, &baseline);
        assert_ne!(result.action, DivergenceAction::Allow);
    }
}
