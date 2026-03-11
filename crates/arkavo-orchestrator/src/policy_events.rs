//! Policy Decision Event Stream for observability
//!
//! Captures every `PolicyDecision` with full `TaskContext` for:
//! - Orchestrator feedback loop (Thompson Sampling)
//! - Anomaly detection (Titan) on decision patterns
//! - Gossip propagation of lessons from policy decisions
//! - Audit trail of all autonomous decisions

use crate::task_policy_manager::{Obligation, PolicyDecision, TaskContext};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// A single policy decision event with full context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEvent {
    pub event_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub task_id: Uuid,
    pub agent_id: String,
    pub tool_name: String,
    pub decision: PolicyDecision,
    pub reasoning: Vec<String>,
    pub entitlements_used: Vec<String>,
    pub estimated_cost: Option<f64>,
    pub obligations: Vec<Obligation>,
}

impl PolicyEvent {
    /// Create a new policy event from a decision context.
    #[must_use]
    pub fn from_decision(
        ctx: &TaskContext,
        decision: &PolicyDecision,
        reasoning: Vec<String>,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            task_id: ctx.task_id,
            agent_id: ctx.agent_id.clone(),
            tool_name: ctx.tool_name.clone(),
            decision: decision.clone(),
            reasoning,
            entitlements_used: ctx.entitlements.clone(),
            estimated_cost: Some(ctx.estimated_cost_dollars),
            obligations: decision.obligations().to_vec(),
        }
    }

    /// Whether this event records a denied decision.
    #[must_use]
    pub fn is_deny(&self) -> bool {
        self.decision.is_denied()
    }

    /// Whether this event records a permitted decision.
    #[must_use]
    pub fn is_permit(&self) -> bool {
        self.decision.is_permitted()
    }
}

/// Callback for policy event consumers.
pub trait PolicyEventHandler: Send + Sync {
    fn on_event(&self, event: &PolicyEvent);
}

/// Policy decision event stream with bounded buffer and handler dispatch.
pub struct PolicyEventStream {
    buffer: Arc<Mutex<VecDeque<PolicyEvent>>>,
    max_buffer_size: usize,
    handlers: Vec<Box<dyn PolicyEventHandler>>,
    stats: Arc<Mutex<StreamStats>>,
}

/// Aggregate statistics for the policy event stream.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamStats {
    pub total_events: u64,
    pub permit_count: u64,
    pub deny_count: u64,
    pub obligation_count: u64,
    pub total_cost: f64,
}

impl StreamStats {
    fn record(&mut self, event: &PolicyEvent) {
        self.total_events += 1;
        match &event.decision {
            PolicyDecision::Permit => self.permit_count += 1,
            PolicyDecision::PermitWithObligations(obs) => {
                self.permit_count += 1;
                self.obligation_count += obs.len() as u64;
            }
            PolicyDecision::Deny(_) => self.deny_count += 1,
        }
        if let Some(cost) = event.estimated_cost {
            if event.decision.is_permitted() {
                self.total_cost += cost;
            }
        }
    }

    /// Deny rate as a fraction (0.0 to 1.0).
    #[must_use]
    pub fn deny_rate(&self) -> f64 {
        if self.total_events == 0 {
            0.0
        } else {
            self.deny_count as f64 / self.total_events as f64
        }
    }
}

impl PolicyEventStream {
    /// Create a new event stream with default buffer size.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(10_000)
    }

    /// Create with a specific buffer capacity.
    #[must_use]
    pub fn with_capacity(max_buffer_size: usize) -> Self {
        Self {
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(
                max_buffer_size.min(1024),
            ))),
            max_buffer_size,
            handlers: Vec::new(),
            stats: Arc::new(Mutex::new(StreamStats::default())),
        }
    }

    /// Register an event handler.
    pub fn add_handler(&mut self, handler: Box<dyn PolicyEventHandler>) {
        self.handlers.push(handler);
    }

    /// Emit a policy event to the stream.
    pub fn emit(&self, event: PolicyEvent) {
        // Dispatch to handlers
        for handler in &self.handlers {
            handler.on_event(&event);
        }

        // Update stats
        if let Ok(mut stats) = self.stats.lock() {
            stats.record(&event);
        }

        // Buffer the event
        if let Ok(mut buffer) = self.buffer.lock() {
            if buffer.len() >= self.max_buffer_size {
                buffer.pop_front();
            }
            buffer.push_back(event);
        }
    }

    /// Emit a policy event directly from a decision context.
    pub fn emit_decision(
        &self,
        ctx: &TaskContext,
        decision: &PolicyDecision,
        reasoning: Vec<String>,
    ) {
        let event = PolicyEvent::from_decision(ctx, decision, reasoning);
        self.emit(event);
    }

    /// Get current stream statistics.
    #[must_use]
    pub fn stats(&self) -> StreamStats {
        self.stats.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Drain recent events from the buffer.
    pub fn drain(&self, limit: usize) -> Vec<PolicyEvent> {
        let mut buffer = match self.buffer.lock() {
            Ok(b) => b,
            Err(_) => return Vec::new(),
        };
        let count = limit.min(buffer.len());
        buffer.drain(..count).collect()
    }

    /// Get recent events without removing them.
    pub fn recent(&self, limit: usize) -> Vec<PolicyEvent> {
        let buffer = match self.buffer.lock() {
            Ok(b) => b,
            Err(_) => return Vec::new(),
        };
        let start = buffer.len().saturating_sub(limit);
        buffer.range(start..).cloned().collect()
    }

    /// Number of buffered events.
    #[must_use]
    pub fn buffered_count(&self) -> usize {
        self.buffer.lock().map(|b| b.len()).unwrap_or(0)
    }
}

impl Default for PolicyEventStream {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn test_ctx(tool: &str) -> TaskContext {
        TaskContext {
            task_id: Uuid::new_v4(),
            agent_id: "agent-1".to_string(),
            entitlements: vec!["ent:read".to_string()],
            tool_name: tool.to_string(),
            tool_params: serde_json::json!({}),
            estimated_cost_dollars: 0.001,
            spawn_depth: 0,
            task_status: "working".to_string(),
        }
    }

    struct CountingHandler {
        count: Arc<AtomicU64>,
    }

    impl PolicyEventHandler for CountingHandler {
        fn on_event(&self, _event: &PolicyEvent) {
            self.count.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn test_emit_and_stats() {
        let stream = PolicyEventStream::new();
        let ctx = test_ctx("ls");
        stream.emit_decision(&ctx, &PolicyDecision::Permit, vec!["all clear".into()]);

        let stats = stream.stats();
        assert_eq!(stats.total_events, 1);
        assert_eq!(stats.permit_count, 1);
        assert_eq!(stats.deny_count, 0);
    }

    #[test]
    fn test_deny_stats() {
        let stream = PolicyEventStream::new();
        let ctx = test_ctx("rm");
        let deny = PolicyDecision::Deny("blocked".into());
        stream.emit_decision(&ctx, &deny, vec!["tool blocked".into()]);

        let stats = stream.stats();
        assert_eq!(stats.deny_count, 1);
        assert!((stats.deny_rate() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_obligation_stats() {
        let stream = PolicyEventStream::new();
        let ctx = test_ctx("shell");
        let decision = PolicyDecision::PermitWithObligations(vec![
            Obligation::AuditLog,
            Obligation::NotifyOrchestrator,
        ]);
        stream.emit_decision(&ctx, &decision, vec![]);

        let stats = stream.stats();
        assert_eq!(stats.obligation_count, 2);
        assert_eq!(stats.permit_count, 1);
    }

    #[test]
    fn test_buffer_drain() {
        let stream = PolicyEventStream::new();
        for i in 0..5 {
            let ctx = test_ctx(&format!("tool_{i}"));
            stream.emit_decision(&ctx, &PolicyDecision::Permit, vec![]);
        }
        assert_eq!(stream.buffered_count(), 5);

        let drained = stream.drain(3);
        assert_eq!(drained.len(), 3);
        assert_eq!(stream.buffered_count(), 2);
    }

    #[test]
    fn test_buffer_overflow() {
        let stream = PolicyEventStream::with_capacity(3);
        for i in 0..5 {
            let ctx = test_ctx(&format!("tool_{i}"));
            stream.emit_decision(&ctx, &PolicyDecision::Permit, vec![]);
        }
        // Buffer capped at 3
        assert_eq!(stream.buffered_count(), 3);
    }

    #[test]
    fn test_recent_events() {
        let stream = PolicyEventStream::new();
        for i in 0..10 {
            let ctx = test_ctx(&format!("tool_{i}"));
            stream.emit_decision(&ctx, &PolicyDecision::Permit, vec![]);
        }
        let recent = stream.recent(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].tool_name, "tool_7");
        assert_eq!(recent[2].tool_name, "tool_9");
    }

    #[test]
    fn test_handler_dispatch() {
        let count = Arc::new(AtomicU64::new(0));
        let mut stream = PolicyEventStream::new();
        stream.add_handler(Box::new(CountingHandler {
            count: count.clone(),
        }));

        let ctx = test_ctx("tool");
        stream.emit_decision(&ctx, &PolicyDecision::Permit, vec![]);
        stream.emit_decision(&ctx, &PolicyDecision::Permit, vec![]);

        assert_eq!(count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_policy_event_creation() {
        let ctx = test_ctx("shell_exec");
        let decision = PolicyDecision::Permit;
        let event = PolicyEvent::from_decision(&ctx, &decision, vec!["ok".into()]);
        assert_eq!(event.tool_name, "shell_exec");
        assert!(event.is_permit());
        assert!(!event.is_deny());
    }

    #[test]
    fn test_deny_rate_empty() {
        let stats = StreamStats::default();
        assert!((stats.deny_rate()).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cost_tracking() {
        let stream = PolicyEventStream::new();
        let ctx = test_ctx("expensive");
        stream.emit_decision(&ctx, &PolicyDecision::Permit, vec![]);
        let deny = PolicyDecision::Deny("no".into());
        stream.emit_decision(&ctx, &deny, vec![]);

        let stats = stream.stats();
        // Only permitted decisions accrue cost
        assert!((stats.total_cost - 0.001).abs() < f64::EPSILON);
    }
}
