use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Subsystem-level timing breakdown for observability.
///
/// Aggregates latency from router decisions, conductor orchestration,
/// MCP tool calls, and LLM inference into a single snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubsystemTiming {
    #[serde(rename = "routerDecisionAvgMs")]
    pub router_decision_avg_ms: f64,
    #[serde(rename = "conductorOrchestrationAvgMs")]
    pub conductor_orchestration_avg_ms: f64,
    #[serde(rename = "mcpToolAvgMs")]
    pub mcp_tool_avg_ms: f64,
    #[serde(rename = "mcpToolP95Ms")]
    pub mcp_tool_p95_ms: f64,
    #[serde(rename = "inferenceAvgMs")]
    pub inference_avg_ms: f64,
}

impl SubsystemTiming {
    pub fn is_empty(&self) -> bool {
        self.router_decision_avg_ms == 0.0
            && self.conductor_orchestration_avg_ms == 0.0
            && self.mcp_tool_avg_ms == 0.0
            && self.inference_avg_ms == 0.0
    }
}

/// Thread-safe rolling window for latency samples.
/// Maintains a fixed-size ring buffer for P50/P95/avg computation.
#[derive(Clone)]
pub struct LatencyTracker {
    inner: Arc<Mutex<LatencyWindow>>,
}

struct LatencyWindow {
    samples: VecDeque<u64>,
    capacity: usize,
    sum: u64,
}

impl LatencyTracker {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LatencyWindow {
                samples: VecDeque::with_capacity(capacity),
                capacity,
                sum: 0,
            })),
        }
    }

    pub fn record(&self, latency_ms: u64) {
        if let Ok(mut w) = self.inner.lock() {
            if w.samples.len() == w.capacity
                && let Some(old) = w.samples.pop_front()
            {
                w.sum = w.sum.saturating_sub(old);
            }
            w.sum += latency_ms;
            w.samples.push_back(latency_ms);
        }
    }

    pub fn avg(&self) -> f64 {
        if let Ok(w) = self.inner.lock() {
            if w.samples.is_empty() {
                0.0
            } else {
                w.sum as f64 / w.samples.len() as f64
            }
        } else {
            0.0
        }
    }

    pub fn p95(&self) -> f64 {
        if let Ok(w) = self.inner.lock() {
            if w.samples.is_empty() {
                return 0.0;
            }
            let mut sorted: Vec<u64> = w.samples.iter().copied().collect();
            sorted.sort_unstable();
            let idx = ((sorted.len() as f64) * 0.95).ceil() as usize;
            sorted[idx.min(sorted.len() - 1)] as f64
        } else {
            0.0
        }
    }
}

/// Global subsystem timing registry.
/// Components record their latencies here; the MetricsSampler reads from it.
pub struct SubsystemTimingRegistry {
    pub router_decisions: LatencyTracker,
    pub conductor_orchestration: LatencyTracker,
    pub mcp_tools: LatencyTracker,
    pub inference: LatencyTracker,
}

impl SubsystemTimingRegistry {
    pub fn new() -> Self {
        Self {
            router_decisions: LatencyTracker::new(50),
            conductor_orchestration: LatencyTracker::new(50),
            mcp_tools: LatencyTracker::new(100),
            inference: LatencyTracker::new(50),
        }
    }

    pub fn snapshot(&self) -> SubsystemTiming {
        SubsystemTiming {
            router_decision_avg_ms: self.router_decisions.avg(),
            conductor_orchestration_avg_ms: self.conductor_orchestration.avg(),
            mcp_tool_avg_ms: self.mcp_tools.avg(),
            mcp_tool_p95_ms: self.mcp_tools.p95(),
            inference_avg_ms: self.inference.avg(),
        }
    }
}

impl Default for SubsystemTimingRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global singleton for subsystem timing.
static GLOBAL_TIMING: std::sync::LazyLock<SubsystemTimingRegistry> =
    std::sync::LazyLock::new(SubsystemTimingRegistry::new);

pub fn global_timing() -> &'static SubsystemTimingRegistry {
    &GLOBAL_TIMING
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latency_tracker_avg() {
        let tracker = LatencyTracker::new(5);
        tracker.record(10);
        tracker.record(20);
        tracker.record(30);
        assert!((tracker.avg() - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_latency_tracker_p95() {
        let tracker = LatencyTracker::new(100);
        for i in 1..=100 {
            tracker.record(i);
        }
        assert!(tracker.p95() >= 95.0);
    }

    #[test]
    fn test_latency_tracker_rolling_window() {
        let tracker = LatencyTracker::new(3);
        tracker.record(10);
        tracker.record(20);
        tracker.record(30);
        tracker.record(40); // pushes out 10
        assert!((tracker.avg() - 30.0).abs() < 0.01);
    }

    #[test]
    fn test_latency_tracker_empty() {
        let tracker = LatencyTracker::new(5);
        assert_eq!(tracker.avg(), 0.0);
        assert_eq!(tracker.p95(), 0.0);
    }

    #[test]
    fn test_subsystem_timing_snapshot() {
        let registry = SubsystemTimingRegistry::new();
        registry.router_decisions.record(15);
        registry.mcp_tools.record(50);
        let snap = registry.snapshot();
        assert!((snap.router_decision_avg_ms - 15.0).abs() < 0.01);
        assert!((snap.mcp_tool_avg_ms - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_subsystem_timing_is_empty() {
        let registry = SubsystemTimingRegistry::new();
        assert!(registry.snapshot().is_empty());
        registry.inference.record(10);
        assert!(!registry.snapshot().is_empty());
    }
}
