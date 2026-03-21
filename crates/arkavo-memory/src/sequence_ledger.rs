use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DataFlowSummary {
    pub source: String,
    pub sink: String,
    pub taint_classification: String,
    pub complete: bool,
}

#[derive(Debug, Clone)]
pub struct LedgerEntry {
    pub agent_id: String,
    pub session_id: String,
    pub timestamp: u64,
    pub data_flows: Vec<DataFlowSummary>,
    pub action_count: usize,
}

pub struct SequenceLedger {
    retention_window: Duration,
    entries: Vec<LedgerEntry>,
}

impl SequenceLedger {
    pub fn new(retention_window: Duration) -> Self {
        Self {
            retention_window,
            entries: Vec::new(),
        }
    }

    pub fn commit(&mut self, entry: LedgerEntry) {
        self.entries.push(entry);
    }

    pub fn entries_for_agent(&self, agent_id: &str) -> Vec<LedgerEntry> {
        self.entries
            .iter()
            .filter(|e| e.agent_id == agent_id)
            .cloned()
            .collect()
    }

    pub fn recent_entries(&self) -> Vec<LedgerEntry> {
        if self.entries.is_empty() {
            return Vec::new();
        }
        let max_ts = self.entries.iter().map(|e| e.timestamp).max().unwrap_or(0);
        let cutoff = max_ts.saturating_sub(self.retention_window.as_secs());
        self.entries
            .iter()
            .filter(|e| e.timestamp >= cutoff)
            .cloned()
            .collect()
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn pending_exfiltration_patterns(&self) -> Vec<DataFlowSummary> {
        self.entries
            .iter()
            .flat_map(|e| e.data_flows.iter())
            .filter(|f| !f.complete)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

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

    #[spec("SEQ-007")]
    #[test]
    fn committed_entry_increments_count() {
        let mut ledger = SequenceLedger::new(Duration::from_secs(86400));
        ledger.commit(sample_entry("agent-1", "session-1"));
        assert_eq!(ledger.entry_count(), 1);
    }

    #[spec("SEQ-007")]
    #[test]
    fn entries_queryable_by_agent_id() {
        let mut ledger = SequenceLedger::new(Duration::from_secs(86400));
        ledger.commit(sample_entry("agent-1", "session-1"));
        let entries = ledger.entries_for_agent("agent-1");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].data_flows.len(), 1);
    }

    #[spec("SEQ-007")]
    #[test]
    fn incomplete_exfiltration_patterns_flagged_as_pending() {
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

    #[spec("SEQ-007")]
    #[test]
    fn multiple_sessions_from_multiple_agents_persisted() {
        let mut ledger = SequenceLedger::new(Duration::from_secs(86400));
        ledger.commit(sample_entry("agent-1", "session-1"));
        ledger.commit(sample_entry("agent-1", "session-2"));
        ledger.commit(sample_entry("agent-2", "session-3"));
        assert_eq!(ledger.entry_count(), 3);
        assert_eq!(ledger.entries_for_agent("agent-1").len(), 2);
    }

    #[spec("SEQ-007")]
    #[test]
    fn entries_outside_retention_window_not_returned() {
        let mut ledger = SequenceLedger::new(Duration::from_secs(100));
        let mut old_entry = sample_entry("agent-1", "old-session");
        old_entry.timestamp = 0;
        ledger.commit(old_entry);
        let mut new_entry = sample_entry("agent-1", "new-session");
        new_entry.timestamp = 500;
        ledger.commit(new_entry);
        let recent = ledger.recent_entries();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].session_id, "new-session");
    }
}
