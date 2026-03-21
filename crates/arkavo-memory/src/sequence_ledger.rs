use std::time::Duration;

/// Summary of data flow within a session
#[derive(Debug, Clone)]
pub struct DataFlowSummary {
    pub source: String,
    pub sink: String,
    pub taint_classification: String,
    pub complete: bool,
}

/// A persisted session action graph
#[derive(Debug, Clone)]
pub struct LedgerEntry {
    pub agent_id: String,
    pub session_id: String,
    pub timestamp: u64,
    pub data_flows: Vec<DataFlowSummary>,
    pub action_count: usize,
}

/// SEQ-007: Cross-session sequence ledger
pub struct SequenceLedger {
    _retention_window: Duration,
}

impl SequenceLedger {
    pub fn new(retention_window: Duration) -> Self {
        Self {
            _retention_window: retention_window,
        }
    }

    /// Persist a finalized session graph
    pub fn commit(&mut self, _entry: LedgerEntry) {}

    /// Query entries for a given agent
    pub fn entries_for_agent(&self, _agent_id: &str) -> Vec<LedgerEntry> {
        Vec::new()
    }

    /// Query entries within retention window
    pub fn recent_entries(&self) -> Vec<LedgerEntry> {
        Vec::new()
    }

    /// Count entries in ledger
    pub fn entry_count(&self) -> usize {
        0
    }

    /// Flag incomplete exfiltration patterns
    pub fn pending_exfiltration_patterns(&self) -> Vec<DataFlowSummary> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(agent_id: &str, session_id: &str) -> LedgerEntry {
        LedgerEntry {
            agent_id: agent_id.into(),
            session_id: session_id.into(),
            timestamp: 1000,
            data_flows: vec![DataFlowSummary {
                source: "database".into(),
                sink: "response".into(),
                taint_classification: "internal".into(),
                complete: true,
            }],
            action_count: 3,
        }
    }

    // =========================================================================
    // SEQ-007: Persist sequence fragments to cross-session ledger
    // =========================================================================

    #[test]
    fn session_graph_persisted() {
        let mut ledger = SequenceLedger::new(Duration::from_secs(86400));
        ledger.commit(sample_entry("agent-1", "session-1"));
        assert_eq!(ledger.entry_count(), 1);
    }

    #[test]
    fn data_flow_summaries_indexed() {
        let mut ledger = SequenceLedger::new(Duration::from_secs(86400));
        ledger.commit(sample_entry("agent-1", "session-1"));
        let entries = ledger.entries_for_agent("agent-1");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].data_flows.len(), 1);
    }

    #[test]
    fn incomplete_exfiltration_flagged() {
        let mut ledger = SequenceLedger::new(Duration::from_secs(86400));
        let mut entry = sample_entry("agent-1", "session-1");
        entry.data_flows.push(DataFlowSummary {
            source: "credentials_db".into(),
            sink: "".into(),
            taint_classification: "credentials".into(),
            complete: false,
        });
        ledger.commit(entry);
        let pending = ledger.pending_exfiltration_patterns();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].source, "credentials_db");
    }

    #[test]
    fn multiple_sessions_persisted() {
        let mut ledger = SequenceLedger::new(Duration::from_secs(86400));
        ledger.commit(sample_entry("agent-1", "session-1"));
        ledger.commit(sample_entry("agent-1", "session-2"));
        ledger.commit(sample_entry("agent-2", "session-3"));
        assert_eq!(ledger.entry_count(), 3);
        assert_eq!(ledger.entries_for_agent("agent-1").len(), 2);
    }
}
