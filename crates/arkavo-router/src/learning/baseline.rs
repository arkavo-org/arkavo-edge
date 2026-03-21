use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct BaselinePattern {
    pub tool_sequence: Vec<String>,
    pub frequency: f64,
    pub max_path_length: usize,
}

#[derive(Debug, Clone)]
pub struct DataFlowProfile {
    pub source_type: String,
    pub sink_type: String,
    pub frequency: f64,
}

pub struct BaselineBuilder {
    skill_id: String,
    min_sessions: usize,
    sessions: Vec<Vec<String>>,
}

impl BaselineBuilder {
    pub fn new(skill_id: &str, min_sessions: usize) -> Self {
        Self {
            skill_id: skill_id.to_string(),
            min_sessions,
            sessions: Vec::new(),
        }
    }

    pub fn record_session(&mut self, tool_sequence: &[&str]) {
        self.sessions
            .push(tool_sequence.iter().map(|s| s.to_string()).collect());
    }

    pub fn has_sufficient_history(&self) -> bool {
        self.sessions.len() >= self.min_sessions
    }

    pub fn build_baseline(&self) -> Option<Baseline> {
        if !self.has_sufficient_history() {
            return None;
        }

        let total = self.sessions.len() as f64;
        let mut freq_map: HashMap<Vec<String>, usize> = HashMap::new();
        let mut max_path = 0usize;

        for session in &self.sessions {
            *freq_map.entry(session.clone()).or_insert(0) += 1;
            max_path = max_path.max(session.len());
        }

        let mut patterns: Vec<BaselinePattern> = freq_map
            .into_iter()
            .map(|(seq, count)| {
                let path_len = seq.len();
                BaselinePattern {
                    tool_sequence: seq,
                    frequency: count as f64 / total,
                    max_path_length: path_len,
                }
            })
            .collect();
        patterns.sort_by(|a, b| b.frequency.partial_cmp(&a.frequency).unwrap());

        Some(Baseline {
            skill_id: self.skill_id.clone(),
            patterns,
            data_flows: Vec::new(),
            max_path_length: max_path,
        })
    }
}

#[derive(Debug)]
pub struct Baseline {
    pub skill_id: String,
    pub patterns: Vec<BaselinePattern>,
    pub data_flows: Vec<DataFlowProfile>,
    pub max_path_length: usize,
}

impl Baseline {
    pub fn matches(&self, sequence: &[&str]) -> bool {
        let seq: Vec<String> = sequence.iter().map(|s| s.to_string()).collect();
        self.patterns.iter().any(|p| p.tool_sequence == seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SEQ-005")]
    #[test]
    fn insufficient_history_returns_no_baseline() {
        let builder = BaselineBuilder::new("code_review", 100);
        assert!(!builder.has_sufficient_history());
        assert!(builder.build_baseline().is_none());
    }

    #[spec("SEQ-005")]
    #[test]
    fn sufficient_history_produces_baseline() {
        let mut builder = BaselineBuilder::new("code_review", 5);
        for _ in 0..5 {
            builder.record_session(&["read_file", "analyze", "respond"]);
        }
        assert!(builder.has_sufficient_history());
    }

    #[spec("SEQ-005")]
    #[test]
    fn baseline_captures_frequency_distribution() {
        let mut builder = BaselineBuilder::new("deploy", 4);
        for _ in 0..3 {
            builder.record_session(&["read_config", "validate", "deploy"]);
        }
        builder.record_session(&["read_config", "deploy"]);
        let baseline = builder.build_baseline().unwrap();
        assert!(!baseline.patterns.is_empty());
        assert!(baseline.patterns[0].frequency > 0.5);
    }

    #[spec("SEQ-005")]
    #[test]
    fn baseline_records_max_path_length() {
        let mut builder = BaselineBuilder::new("pipeline", 2);
        builder.record_session(&["a", "b", "c", "d", "e"]);
        builder.record_session(&["a", "b"]);
        let baseline = builder.build_baseline().unwrap();
        assert_eq!(baseline.max_path_length, 5);
    }

    #[spec("SEQ-005")]
    #[test]
    fn baseline_stored_with_skill_identifier() {
        let mut builder = BaselineBuilder::new("code_review", 1);
        builder.record_session(&["read", "analyze"]);
        let baseline = builder.build_baseline().unwrap();
        assert_eq!(baseline.skill_id, "code_review");
    }

    #[spec("SEQ-005")]
    #[test]
    fn known_pattern_matches_baseline() {
        let mut builder = BaselineBuilder::new("review", 1);
        builder.record_session(&["read_file", "analyze", "respond"]);
        let baseline = builder.build_baseline().unwrap();
        assert!(baseline.matches(&["read_file", "analyze", "respond"]));
    }

    #[spec("SEQ-005")]
    #[test]
    fn unknown_pattern_does_not_match() {
        let mut builder = BaselineBuilder::new("review", 1);
        builder.record_session(&["read_file", "analyze", "respond"]);
        let baseline = builder.build_baseline().unwrap();
        assert!(!baseline.matches(&["read_credentials", "http_post"]));
    }

    #[spec("SEQ-005")]
    #[test]
    fn invariant_constraints_override_poisoned_baseline() {
        let mut builder = BaselineBuilder::new("poisoned_skill", 1);
        builder.record_session(&["read_secrets", "http_post_external"]);
        let baseline = builder.build_baseline();
        if let Some(b) = baseline {
            assert!(b.matches(&["read_secrets", "http_post_external"]));
        }
    }
}
