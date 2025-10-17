use crate::engine::PipelineStatus;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineMetrics {
    pub pipeline_id: Uuid,
    pub pipeline_name: String,
    pub status: PipelineStatus,
    pub node_metrics: HashMap<String, crate::engine::executor::NodeMetrics>,
    pub timestamp: DateTime<Utc>,
}

impl PipelineMetrics {
    pub fn total_messages_processed(&self) -> u64 {
        self.node_metrics
            .values()
            .map(|m| m.messages_received)
            .sum()
    }

    pub fn total_messages_sent(&self) -> u64 {
        self.node_metrics.values().map(|m| m.messages_sent).sum()
    }

    pub fn total_messages_filtered(&self) -> u64 {
        self.node_metrics
            .values()
            .map(|m| m.messages_filtered)
            .sum()
    }

    pub fn total_errors(&self) -> u64 {
        self.node_metrics.values().map(|m| m.messages_errored).sum()
    }

    pub fn average_processing_time_ms(&self) -> f64 {
        let total_time: u64 = self
            .node_metrics
            .values()
            .map(|m| m.processing_time_ms)
            .sum();

        let total_messages = self.total_messages_processed();

        if total_messages > 0 {
            total_time as f64 / total_messages as f64
        } else {
            0.0
        }
    }

    pub fn bottleneck_nodes(&self, threshold: usize) -> Vec<String> {
        self.node_metrics
            .iter()
            .filter(|(_, metrics)| metrics.queue_depth > threshold)
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub fn error_rate(&self) -> f64 {
        let total_processed = self.total_messages_processed();
        if total_processed > 0 {
            self.total_errors() as f64 / total_processed as f64
        } else {
            0.0
        }
    }

    pub fn throughput_per_second(&self, duration_seconds: f64) -> f64 {
        if duration_seconds > 0.0 {
            self.total_messages_processed() as f64 / duration_seconds
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub pipelines: Vec<PipelineMetrics>,
    pub system_metrics: SystemMetrics,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub total_pipelines: usize,
    pub running_pipelines: usize,
    pub total_messages_processed: u64,
    pub cpu_usage_percent: Option<f32>,
    pub memory_usage_mb: Option<u64>,
}

pub struct MetricsCollector {
    snapshots: Vec<MetricsSnapshot>,
    max_snapshots: usize,
}

impl MetricsCollector {
    pub fn new(max_snapshots: usize) -> Self {
        Self {
            snapshots: Vec::with_capacity(max_snapshots),
            max_snapshots,
        }
    }

    pub fn record_snapshot(&mut self, snapshot: MetricsSnapshot) {
        if self.snapshots.len() >= self.max_snapshots {
            self.snapshots.remove(0);
        }
        self.snapshots.push(snapshot);
    }

    pub fn get_snapshots(&self) -> &[MetricsSnapshot] {
        &self.snapshots
    }

    pub fn get_latest(&self) -> Option<&MetricsSnapshot> {
        self.snapshots.last()
    }

    pub fn calculate_trends(&self, window_size: usize) -> Option<MetricsTrend> {
        if self.snapshots.len() < window_size {
            return None;
        }

        let recent = &self.snapshots[self.snapshots.len() - window_size..];

        let throughput_trend = self.calculate_throughput_trend(recent);
        let error_trend = self.calculate_error_trend(recent);
        let latency_trend = self.calculate_latency_trend(recent);

        Some(MetricsTrend {
            throughput_trend,
            error_trend,
            latency_trend,
            window_size,
        })
    }

    fn calculate_throughput_trend(&self, snapshots: &[MetricsSnapshot]) -> TrendDirection {
        if snapshots.len() < 2 {
            return TrendDirection::Stable;
        }

        let first_throughput = snapshots
            .first()
            .map_or(0, |s| s.system_metrics.total_messages_processed)
            as f64;

        let last_throughput = snapshots
            .last()
            .map_or(0, |s| s.system_metrics.total_messages_processed)
            as f64;

        let change_percent = if first_throughput > 0.0 {
            ((last_throughput - first_throughput) / first_throughput) * 100.0
        } else {
            0.0
        };

        match change_percent {
            p if p > 10.0 => TrendDirection::Increasing(p),
            p if p < -10.0 => TrendDirection::Decreasing(p.abs()),
            _ => TrendDirection::Stable,
        }
    }

    fn calculate_error_trend(&self, snapshots: &[MetricsSnapshot]) -> TrendDirection {
        if snapshots.len() < 2 {
            return TrendDirection::Stable;
        }

        let error_rates: Vec<f64> = snapshots
            .iter()
            .map(|s| {
                let total_errors: u64 = s.pipelines.iter().map(PipelineMetrics::total_errors).sum();
                let total_processed = s.system_metrics.total_messages_processed;

                if total_processed > 0 {
                    total_errors as f64 / total_processed as f64
                } else {
                    0.0
                }
            })
            .collect();

        let first_rate = error_rates.first().copied().unwrap_or(0.0);
        let last_rate = error_rates.last().copied().unwrap_or(0.0);

        let change = last_rate - first_rate;

        match change {
            c if c > 0.01 => TrendDirection::Increasing(c * 100.0),
            c if c < -0.01 => TrendDirection::Decreasing(c.abs() * 100.0),
            _ => TrendDirection::Stable,
        }
    }

    fn calculate_latency_trend(&self, _snapshots: &[MetricsSnapshot]) -> TrendDirection {
        // Similar implementation for latency trends
        TrendDirection::Stable
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsTrend {
    pub throughput_trend: TrendDirection,
    pub error_trend: TrendDirection,
    pub latency_trend: TrendDirection,
    pub window_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrendDirection {
    Increasing(f64), // Percentage
    Decreasing(f64), // Percentage
    Stable,
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new(100)
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_metrics_calculations() {
        let mut node_metrics = HashMap::new();
        node_metrics.insert(
            "node1".to_string(),
            crate::engine::executor::NodeMetrics {
                messages_received: 100,
                messages_sent: 90,
                messages_filtered: 10,
                messages_errored: 5,
                processing_time_ms: 1000,
                queue_depth: 0,
            },
        );
        node_metrics.insert(
            "node2".to_string(),
            crate::engine::executor::NodeMetrics {
                messages_received: 90,
                messages_sent: 85,
                messages_filtered: 5,
                messages_errored: 0,
                processing_time_ms: 500,
                queue_depth: 10,
            },
        );

        let metrics = PipelineMetrics {
            pipeline_id: Uuid::new_v4(),
            pipeline_name: "test".to_string(),
            status: PipelineStatus::Running,
            node_metrics,
            timestamp: Utc::now(),
        };

        assert_eq!(metrics.total_messages_processed(), 190);
        assert_eq!(metrics.total_messages_sent(), 175);
        assert_eq!(metrics.total_errors(), 5);

        let avg_time = metrics.average_processing_time_ms();
        assert!((avg_time - 7.89).abs() < 0.1); // ~1500/190

        let bottlenecks = metrics.bottleneck_nodes(5);
        assert_eq!(bottlenecks.len(), 1);
        assert!(bottlenecks.contains(&"node2".to_string()));
    }
}
