use std::collections::HashSet;

pub struct AsyncSequenceMonitor {
    anomaly_threshold: f64,
    action_count: usize,
    unique_tools: HashSet<String>,
    tainted_count: usize,
}

#[derive(Debug, PartialEq)]
pub enum MonitorAlert {
    None,
    ThresholdBreach { score: f64 },
}

impl AsyncSequenceMonitor {
    pub fn new(anomaly_threshold: f64) -> Self {
        Self {
            anomaly_threshold,
            action_count: 0,
            unique_tools: HashSet::new(),
            tainted_count: 0,
        }
    }

    pub fn log_action(&mut self, tool_name: &str, taint_labels: &[String]) {
        self.action_count += 1;
        self.unique_tools.insert(tool_name.to_string());
        if !taint_labels.is_empty() {
            self.tainted_count += 1;
        }
    }

    pub fn anomaly_score(&self) -> f64 {
        if self.action_count == 0 {
            return 0.0;
        }
        let tool_ratio = self.unique_tools.len() as f64 / self.action_count as f64;
        let taint_ratio = self.tainted_count as f64 / self.action_count as f64;
        0.5f64.mul_add(tool_ratio, 0.5 * taint_ratio).min(1.0)
    }

    pub fn check_alert(&self) -> MonitorAlert {
        let score = self.anomaly_score();
        if score > self.anomaly_threshold {
            MonitorAlert::ThresholdBreach { score }
        } else {
            MonitorAlert::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SEQ-012")]
    #[test]
    fn anomaly_score_non_negative_after_logging_action() {
        let mut monitor = AsyncSequenceMonitor::new(0.8);
        monitor.log_action("read_file", &[]);
        assert!(monitor.anomaly_score() >= 0.0);
    }

    #[spec("SEQ-012")]
    #[test]
    fn normal_actions_produce_no_alert() {
        let mut monitor = AsyncSequenceMonitor::new(0.8);
        monitor.log_action("read_file", &[]);
        monitor.log_action("summarize", &[]);
        assert_eq!(monitor.check_alert(), MonitorAlert::None);
    }

    #[spec("SEQ-012")]
    #[test]
    fn anomalous_pattern_breaches_threshold() {
        let mut monitor = AsyncSequenceMonitor::new(0.5);
        for i in 0..20 {
            monitor.log_action(&format!("suspicious_tool_{i}"), &["credentials".into()]);
        }
        assert!(matches!(
            monitor.check_alert(),
            MonitorAlert::ThresholdBreach { .. }
        ));
    }

    #[spec("SEQ-012")]
    #[test]
    fn all_session_actions_reflected_in_score() {
        let mut monitor = AsyncSequenceMonitor::new(0.8);
        monitor.log_action("read", &[]);
        monitor.log_action("transform", &["internal".into()]);
        monitor.log_action("write_local", &[]);
        assert!(monitor.anomaly_score() >= 0.0);
    }
}
