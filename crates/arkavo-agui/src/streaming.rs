use arkavo_protocol::types::MessageDelta;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Enhanced message delta with ordering guarantees
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderedMessageDelta {
    /// Original message delta
    #[serde(flatten)]
    pub delta: MessageDelta,

    /// Sequence number for ordering within a message
    pub sequence: u64,

    /// Trace ID for latency measurement across hops
    pub trace_id: String,

    /// Agent ID this delta is from
    pub agent_id: String,
}

/// Manages ordered delivery of message deltas
pub struct StreamOrdering {
    /// Expected sequence numbers per message_id
    expected_sequences: Arc<RwLock<HashMap<String, u64>>>,

    /// Buffer for out-of-order deltas
    pending_deltas: Arc<RwLock<HashMap<String, Vec<OrderedMessageDelta>>>>,
}

impl Default for StreamOrdering {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamOrdering {
    pub fn new() -> Self {
        Self {
            expected_sequences: Arc::new(RwLock::new(HashMap::new())),
            pending_deltas: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Process a delta and return ordered deltas ready for delivery
    pub async fn process_delta(&self, delta: OrderedMessageDelta) -> Vec<OrderedMessageDelta> {
        let message_id = delta.delta.message_id.clone();
        let sequence = delta.sequence;

        let mut sequences = self.expected_sequences.write().await;
        let mut pending = self.pending_deltas.write().await;

        // Get expected sequence for this message
        let expected = sequences.entry(message_id.clone()).or_insert(0);

        let mut ready_deltas = Vec::new();

        if sequence == *expected {
            // This is the expected delta
            ready_deltas.push(delta);
            *expected += 1;

            // Check if we have any pending deltas that are now ready
            if let Some(pending_list) = pending.get_mut(&message_id) {
                // Sort by sequence
                pending_list.sort_by_key(|d| d.sequence);

                // Take all consecutive deltas
                while let Some(next_delta) = pending_list.first() {
                    if next_delta.sequence == *expected {
                        ready_deltas.push(pending_list.remove(0));
                        *expected += 1;
                    } else {
                        break;
                    }
                }

                // Remove empty pending list
                if pending_list.is_empty() {
                    pending.remove(&message_id);
                }
            }
        } else if sequence > *expected {
            // Out of order - buffer it
            pending
                .entry(message_id)
                .or_insert_with(Vec::new)
                .push(delta);
        }
        // Ignore deltas with sequence < expected (duplicates)

        ready_deltas
    }

    /// Clean up state for a completed message
    pub async fn complete_message(&self, message_id: &str) {
        self.expected_sequences.write().await.remove(message_id);
        self.pending_deltas.write().await.remove(message_id);
    }
}

/// Tracks latency metrics for message routing
pub struct LatencyTracker {
    /// Start times for each trace_id
    trace_starts: Arc<RwLock<HashMap<String, chrono::DateTime<chrono::Utc>>>>,

    /// Hop latencies for each trace_id
    hop_latencies: Arc<RwLock<HashMap<String, Vec<HopLatency>>>>,
}

#[derive(Debug, Clone)]
pub struct HopLatency {
    pub hop_name: String,
    pub duration_ms: u64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl Default for LatencyTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl LatencyTracker {
    pub fn new() -> Self {
        Self {
            trace_starts: Arc::new(RwLock::new(HashMap::new())),
            hop_latencies: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start tracking a new trace
    pub async fn start_trace(&self, trace_id: String) {
        self.trace_starts
            .write()
            .await
            .insert(trace_id, chrono::Utc::now());
    }

    /// Record a hop in the trace
    pub async fn record_hop(&self, trace_id: &str, hop_name: &str) {
        let now = chrono::Utc::now();

        // Calculate duration from start
        let duration_ms = if let Some(start) = self.trace_starts.read().await.get(trace_id) {
            (now - *start).num_milliseconds() as u64
        } else {
            0
        };

        let hop = HopLatency {
            hop_name: hop_name.to_string(),
            duration_ms,
            timestamp: now,
        };

        self.hop_latencies
            .write()
            .await
            .entry(trace_id.to_string())
            .or_insert_with(Vec::new)
            .push(hop);
    }

    /// Get total latency for a trace
    pub async fn get_total_latency(&self, trace_id: &str) -> Option<u64> {
        if let Some(start) = self.trace_starts.read().await.get(trace_id) {
            let now = chrono::Utc::now();
            Some((now - *start).num_milliseconds() as u64)
        } else {
            None
        }
    }

    /// Get all hop latencies for a trace
    pub async fn get_hop_latencies(&self, trace_id: &str) -> Vec<HopLatency> {
        self.hop_latencies
            .read()
            .await
            .get(trace_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Clean up completed trace
    pub async fn complete_trace(&self, trace_id: &str) {
        self.trace_starts.write().await.remove(trace_id);
        self.hop_latencies.write().await.remove(trace_id);
    }
}

/// Converts between protocol MessageDelta and OrderedMessageDelta
pub fn to_ordered_delta(
    delta: MessageDelta,
    sequence: u64,
    trace_id: String,
    agent_id: String,
) -> OrderedMessageDelta {
    OrderedMessageDelta {
        delta,
        sequence,
        trace_id,
        agent_id,
    }
}

/// Converts from OrderedMessageDelta back to protocol MessageDelta
pub fn from_ordered_delta(ordered: OrderedMessageDelta) -> MessageDelta {
    ordered.delta
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::types::MessageDeltaContent;

    #[tokio::test]
    async fn test_stream_ordering() {
        let ordering = StreamOrdering::new();
        let message_id = "test-message".to_string();

        // Create deltas out of order
        let delta1 = OrderedMessageDelta {
            delta: MessageDelta {
                session_id: "session1".to_string(),
                message_id: message_id.clone(),
                sequence: 0,
                delta: MessageDeltaContent::Text {
                    text: "First".to_string(),
                },
                timestamp: chrono::Utc::now(),
            },
            sequence: 0,
            trace_id: "trace1".to_string(),
            agent_id: "agent1".to_string(),
        };

        let delta2 = OrderedMessageDelta {
            delta: MessageDelta {
                session_id: "session1".to_string(),
                message_id: message_id.clone(),
                sequence: 1,
                delta: MessageDeltaContent::Text {
                    text: "Second".to_string(),
                },
                timestamp: chrono::Utc::now(),
            },
            sequence: 1,
            trace_id: "trace1".to_string(),
            agent_id: "agent1".to_string(),
        };

        let delta3 = OrderedMessageDelta {
            delta: MessageDelta {
                session_id: "session1".to_string(),
                message_id: message_id.clone(),
                sequence: 2,
                delta: MessageDeltaContent::Text {
                    text: "Third".to_string(),
                },
                timestamp: chrono::Utc::now(),
            },
            sequence: 2,
            trace_id: "trace1".to_string(),
            agent_id: "agent1".to_string(),
        };

        // Process out of order: 2, 0, 1
        let ready = ordering.process_delta(delta3.clone()).await;
        assert_eq!(ready.len(), 0); // Delta 2 is buffered

        let ready = ordering.process_delta(delta1.clone()).await;
        assert_eq!(ready.len(), 1); // Delta 0 is delivered
        assert_eq!(ready[0].sequence, 0);

        let ready = ordering.process_delta(delta2.clone()).await;
        assert_eq!(ready.len(), 2); // Deltas 1 and 2 are delivered
        assert_eq!(ready[0].sequence, 1);
        assert_eq!(ready[1].sequence, 2);
    }

    #[tokio::test]
    async fn test_latency_tracking() {
        let tracker = LatencyTracker::new();
        let trace_id = "test-trace".to_string();

        tracker.start_trace(trace_id.clone()).await;

        // Simulate hops with delays
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        tracker.record_hop(&trace_id, "gateway").await;

        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        tracker.record_hop(&trace_id, "agent").await;

        let hops = tracker.get_hop_latencies(&trace_id).await;
        assert_eq!(hops.len(), 2);
        assert_eq!(hops[0].hop_name, "gateway");
        assert_eq!(hops[1].hop_name, "agent");
        assert!(hops[1].duration_ms > hops[0].duration_ms);

        let total = tracker.get_total_latency(&trace_id).await.unwrap();
        assert!(total >= 30); // At least 30ms total
    }
}
