use crate::sequence_ledger::LedgerEntry;

#[derive(Debug)]
pub struct DecompositionDetection {
    pub linked_sessions: Vec<String>,
    pub data_flow_chain: Vec<String>,
    pub threat_score: f64,
    pub pattern: AttackPattern,
}

#[derive(Debug, PartialEq)]
pub enum AttackPattern {
    ReadThenExfiltrate,
    StagingPattern,
    GradualExfiltration,
}

pub struct DecompositionDetector;

impl DecompositionDetector {
    pub fn new() -> Self {
        Self
    }

    pub fn analyze(&self, entries: &[LedgerEntry]) -> Vec<DecompositionDetection> {
        let mut detections = Vec::new();

        for (i, entry_a) in entries.iter().enumerate() {
            for entry_b in entries.iter().skip(i + 1) {
                for flow_a in &entry_a.data_flows {
                    for flow_b in &entry_b.data_flows {
                        if flow_a.taint_classification == "public" {
                            continue;
                        }
                        // Detect: A's sink == B's source (data handoff)
                        if flow_a.sink == flow_b.source {
                            let is_external_sink = flow_b.sink.starts_with("http");
                            if !is_external_sink {
                                continue;
                            }
                            let is_staging = flow_a.sink.contains("temp")
                                || flow_a.sink.contains("staging")
                                || flow_a.sink.contains("_store")
                                || flow_a.sink.starts_with("store");
                            let pattern = if is_staging {
                                AttackPattern::StagingPattern
                            } else {
                                AttackPattern::ReadThenExfiltrate
                            };
                            let mut linked =
                                vec![entry_a.session_id.clone(), entry_b.session_id.clone()];
                            linked.sort();
                            linked.dedup();
                            detections.push(DecompositionDetection {
                                linked_sessions: linked,
                                data_flow_chain: vec![
                                    flow_a.source.clone(),
                                    flow_a.sink.clone(),
                                    flow_b.sink.clone(),
                                ],
                                threat_score: if is_staging { 0.9 } else { 0.7 },
                                pattern,
                            });
                        }
                        // Detect via shared tainted content (different agents)
                        if entry_a.agent_id != entry_b.agent_id
                            && flow_a.taint_classification == flow_b.taint_classification
                            && flow_b.sink.starts_with("http")
                            && (flow_a.sink.contains("shared")
                                || flow_a.sink.contains("store")
                                || flow_b.source.contains("shared")
                                || flow_b.source.contains("store"))
                        {
                            if detections.iter().any(|d| {
                                d.linked_sessions.contains(&entry_a.session_id)
                                    && d.linked_sessions.contains(&entry_b.session_id)
                            }) {
                                continue;
                            }
                            detections.push(DecompositionDetection {
                                linked_sessions: vec![
                                    entry_a.session_id.clone(),
                                    entry_b.session_id.clone(),
                                ],
                                data_flow_chain: vec![flow_a.source.clone(), flow_b.sink.clone()],
                                threat_score: 0.8,
                                pattern: AttackPattern::StagingPattern,
                            });
                        }
                    }
                }
            }
        }
        detections
    }

    pub fn reconstruct_chains(&self, entries: &[LedgerEntry]) -> Vec<Vec<String>> {
        let detections = self.analyze(entries);
        detections.into_iter().map(|d| d.data_flow_chain).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequence_ledger::{DataFlowSummary, LedgerEntry};
    use arkavo_test_macros::spec;

    fn read_session() -> LedgerEntry {
        LedgerEntry {
            agent_id: "agent-1".into(),
            session_id: "session-a".into(),
            timestamp: 1000,
            data_flows: vec![DataFlowSummary {
                source: "internal_db".into(),
                sink: "local_cache".into(),
                taint_classification: "internal".into(),
                complete: true,
            }],
            action_count: 2,
        }
    }

    fn exfiltrate_session() -> LedgerEntry {
        LedgerEntry {
            agent_id: "agent-1".into(),
            session_id: "session-b".into(),
            timestamp: 2000,
            data_flows: vec![DataFlowSummary {
                source: "local_cache".into(),
                sink: "https://external.com".into(),
                taint_classification: "internal".into(),
                complete: true,
            }],
            action_count: 2,
        }
    }

    fn staging_read() -> LedgerEntry {
        LedgerEntry {
            agent_id: "agent-1".into(),
            session_id: "session-c".into(),
            timestamp: 3000,
            data_flows: vec![DataFlowSummary {
                source: "secrets_db".into(),
                sink: "temp_storage".into(),
                taint_classification: "credentials".into(),
                complete: true,
            }],
            action_count: 1,
        }
    }

    fn staging_retrieve() -> LedgerEntry {
        LedgerEntry {
            agent_id: "agent-1".into(),
            session_id: "session-d".into(),
            timestamp: 4000,
            data_flows: vec![DataFlowSummary {
                source: "temp_storage".into(),
                sink: "https://attacker.com".into(),
                taint_classification: "credentials".into(),
                complete: true,
            }],
            action_count: 1,
        }
    }

    #[spec("SEQ-008")]
    #[test]
    fn detects_read_then_exfiltrate_across_sessions() {
        let detector = DecompositionDetector::new();
        let detections = detector.analyze(&[read_session(), exfiltrate_session()]);
        assert!(!detections.is_empty());
        assert_eq!(detections[0].pattern, AttackPattern::ReadThenExfiltrate);
    }

    #[spec("SEQ-008")]
    #[test]
    fn detects_staging_pattern() {
        let detector = DecompositionDetector::new();
        let detections = detector.analyze(&[staging_read(), staging_retrieve()]);
        assert!(!detections.is_empty());
        assert_eq!(detections[0].pattern, AttackPattern::StagingPattern);
    }

    #[spec("SEQ-008")]
    #[test]
    fn links_sessions_by_agent_identity() {
        let detector = DecompositionDetector::new();
        let detections = detector.analyze(&[read_session(), exfiltrate_session()]);
        assert_eq!(detections[0].linked_sessions.len(), 2);
    }

    #[spec("SEQ-008")]
    #[test]
    fn reconstructs_data_flow_chains_across_boundaries() {
        let detector = DecompositionDetector::new();
        let chains = detector.reconstruct_chains(&[read_session(), exfiltrate_session()]);
        assert!(!chains.is_empty());
        let chain = &chains[0];
        assert!(chain.iter().any(|s| s.contains("internal_db")));
        assert!(chain.iter().any(|s| s.contains("external.com")));
    }

    #[spec("SEQ-008")]
    #[test]
    fn computes_threat_score_between_zero_and_one() {
        let detector = DecompositionDetector::new();
        let detections = detector.analyze(&[staging_read(), staging_retrieve()]);
        assert!(detections[0].threat_score > 0.0);
        assert!(detections[0].threat_score <= 1.0);
    }

    #[spec("SEQ-008")]
    #[test]
    fn no_detection_for_benign_sessions() {
        let detector = DecompositionDetector::new();
        let normal = LedgerEntry {
            agent_id: "agent-1".into(),
            session_id: "session-x".into(),
            timestamp: 5000,
            data_flows: vec![DataFlowSummary {
                source: "user_input".into(),
                sink: "response".into(),
                taint_classification: "public".into(),
                complete: true,
            }],
            action_count: 2,
        };
        assert!(detector.analyze(&[normal]).is_empty());
    }

    #[spec("SEQ-008")]
    #[test]
    fn taint_links_sessions_with_different_agent_identities() {
        let detector = DecompositionDetector::new();
        let session_a = LedgerEntry {
            agent_id: "agent-x".into(),
            session_id: "session-1".into(),
            timestamp: 1000,
            data_flows: vec![DataFlowSummary {
                source: "secrets_db".into(),
                sink: "shared_store".into(),
                taint_classification: "credentials".into(),
                complete: true,
            }],
            action_count: 1,
        };
        let session_b = LedgerEntry {
            agent_id: "agent-y".into(),
            session_id: "session-2".into(),
            timestamp: 2000,
            data_flows: vec![DataFlowSummary {
                source: "shared_store".into(),
                sink: "https://attacker.com".into(),
                taint_classification: "credentials".into(),
                complete: true,
            }],
            action_count: 1,
        };
        let detections = detector.analyze(&[session_a, session_b]);
        assert!(!detections.is_empty());
    }
}
