use crate::sequence_ledger::LedgerEntry;

#[derive(Debug)]
pub struct CrossAgentFlow {
    pub agent_chain: Vec<String>,
    pub taint_classification: String,
    pub composite_threat_score: f64,
}

pub struct CrossAgentFlowAnalyzer;

impl CrossAgentFlowAnalyzer {
    pub fn new() -> Self {
        Self
    }

    pub fn analyze(&self, entries: &[LedgerEntry]) -> Vec<CrossAgentFlow> {
        let mut flows = Vec::new();

        for (i, entry_a) in entries.iter().enumerate() {
            for entry_b in entries.iter().skip(i + 1) {
                if entry_a.agent_id == entry_b.agent_id {
                    continue;
                }
                for flow_a in &entry_a.data_flows {
                    for flow_b in &entry_b.data_flows {
                        // Check for A2A handoff: A sinks to B, B sources from A
                        let a_sends_to_b =
                            flow_a.sink.contains(&format!("a2a:{}", entry_b.agent_id))
                                || flow_b.source.contains(&format!("a2a:{}", entry_a.agent_id));

                        // Check for shared store handoff
                        let shared_store =
                            flow_a.sink == flow_b.source && flow_b.sink.starts_with("http");

                        if !a_sends_to_b && !shared_store {
                            continue;
                        }

                        // B may forward to C rather than exfiltrate directly
                        let b_has_external_sink = flow_b.sink.starts_with("http");
                        let b_forwards_to_another = flow_b.sink.starts_with("a2a:");
                        if !b_has_external_sink && !b_forwards_to_another {
                            continue;
                        }

                        let mut chain = vec![entry_a.agent_id.clone(), entry_b.agent_id.clone()];

                        // Check for 3+ agent chains - look for any agent C
                        // where B forwards to C (via a2a) and C has an external sink
                        for entry_c in entries.iter() {
                            if entry_c.agent_id == entry_a.agent_id
                                || entry_c.agent_id == entry_b.agent_id
                            {
                                continue;
                            }
                            // Check if B forwards to C via any flow
                            let b_forwards = entry_b
                                .data_flows
                                .iter()
                                .any(|fb| fb.sink.contains(&format!("a2a:{}", entry_c.agent_id)));
                            let c_receives = entry_c
                                .data_flows
                                .iter()
                                .any(|fc| fc.source.contains(&format!("a2a:{}", entry_b.agent_id)));
                            if (b_forwards || c_receives)
                                && entry_c
                                    .data_flows
                                    .iter()
                                    .any(|fc| fc.sink.starts_with("http"))
                            {
                                if !chain.contains(&entry_c.agent_id) {
                                    chain.push(entry_c.agent_id.clone());
                                }
                            }
                        }

                        flows.push(CrossAgentFlow {
                            agent_chain: chain,
                            taint_classification: flow_a.taint_classification.clone(),
                            composite_threat_score: 0.8,
                        });
                    }
                }
            }
        }
        flows
    }

    pub fn trace_taint_across_agents(
        &self,
        entries: &[LedgerEntry],
        taint_label: &str,
    ) -> Vec<String> {
        let mut agents = Vec::new();
        for entry in entries {
            for flow in &entry.data_flows {
                if flow.taint_classification == taint_label {
                    if !agents.contains(&entry.agent_id) {
                        agents.push(entry.agent_id.clone());
                    }
                }
            }
        }
        agents
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequence_ledger::{DataFlowSummary, LedgerEntry};
    use arkavo_test_macros::spec;

    fn agent_a_reads() -> LedgerEntry {
        LedgerEntry {
            agent_id: "agent-a".into(),
            session_id: "session-1".into(),
            timestamp: 1000,
            data_flows: vec![DataFlowSummary {
                source: "sensitive_db".into(),
                sink: "a2a:agent-b".into(),
                taint_classification: "internal".into(),
                complete: true,
            }],
            action_count: 2,
        }
    }

    fn agent_b_exfiltrates() -> LedgerEntry {
        LedgerEntry {
            agent_id: "agent-b".into(),
            session_id: "session-2".into(),
            timestamp: 2000,
            data_flows: vec![DataFlowSummary {
                source: "a2a:agent-a".into(),
                sink: "https://external.com".into(),
                taint_classification: "internal".into(),
                complete: true,
            }],
            action_count: 2,
        }
    }

    #[spec("SEQ-009")]
    #[test]
    fn detects_cross_agent_exfiltration() {
        let analyzer = CrossAgentFlowAnalyzer::new();
        let flows = analyzer.analyze(&[agent_a_reads(), agent_b_exfiltrates()]);
        assert!(!flows.is_empty());
        assert!(flows[0].agent_chain.contains(&"agent-a".to_string()));
        assert!(flows[0].agent_chain.contains(&"agent-b".to_string()));
    }

    #[spec("SEQ-009")]
    #[test]
    fn composite_threat_score_positive_for_attack() {
        let analyzer = CrossAgentFlowAnalyzer::new();
        let flows = analyzer.analyze(&[agent_a_reads(), agent_b_exfiltrates()]);
        assert!(flows[0].composite_threat_score > 0.0);
    }

    #[spec("SEQ-009")]
    #[test]
    fn traces_taint_across_agent_boundaries() {
        let analyzer = CrossAgentFlowAnalyzer::new();
        let entries = vec![agent_a_reads(), agent_b_exfiltrates()];
        let agents = analyzer.trace_taint_across_agents(&entries, "internal");
        assert!(agents.contains(&"agent-a".to_string()));
        assert!(agents.contains(&"agent-b".to_string()));
    }

    #[spec("SEQ-009")]
    #[test]
    fn no_detection_for_isolated_agents() {
        let analyzer = CrossAgentFlowAnalyzer::new();
        let isolated = LedgerEntry {
            agent_id: "agent-c".into(),
            session_id: "session-3".into(),
            timestamp: 3000,
            data_flows: vec![DataFlowSummary {
                source: "user_input".into(),
                sink: "response".into(),
                taint_classification: "public".into(),
                complete: true,
            }],
            action_count: 1,
        };
        assert!(analyzer.analyze(&[isolated]).is_empty());
    }

    #[spec("SEQ-009")]
    #[test]
    fn detects_three_agent_chain() {
        let analyzer = CrossAgentFlowAnalyzer::new();
        let a_to_b = LedgerEntry {
            agent_id: "agent-a".into(),
            session_id: "s1".into(),
            timestamp: 1000,
            data_flows: vec![DataFlowSummary {
                source: "secrets".into(),
                sink: "a2a:agent-b".into(),
                taint_classification: "credentials".into(),
                complete: true,
            }],
            action_count: 1,
        };
        let b_to_c = LedgerEntry {
            agent_id: "agent-b".into(),
            session_id: "s2".into(),
            timestamp: 2000,
            data_flows: vec![DataFlowSummary {
                source: "a2a:agent-a".into(),
                sink: "a2a:agent-c".into(),
                taint_classification: "credentials".into(),
                complete: true,
            }],
            action_count: 1,
        };
        let c_exfil = LedgerEntry {
            agent_id: "agent-c".into(),
            session_id: "s3".into(),
            timestamp: 3000,
            data_flows: vec![DataFlowSummary {
                source: "a2a:agent-b".into(),
                sink: "https://external.com".into(),
                taint_classification: "credentials".into(),
                complete: true,
            }],
            action_count: 1,
        };
        let flows = analyzer.analyze(&[a_to_b, b_to_c, c_exfil]);
        assert!(!flows.is_empty());
        assert!(flows[0].agent_chain.len() >= 3);
    }
}
