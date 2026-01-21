// Test utilities for time sync stress/load tests
// These may not all be used in every test file
#![allow(dead_code)]

use arkavo_mcp_tools::{time_sync::SyncAgentTimeTool, Tool};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Semaphore;

pub struct AgentSimulator {
    pub id: String,
    pub sync_tool: SyncAgentTimeTool,
}

impl AgentSimulator {
    pub fn new(id: String) -> Self {
        Self {
            id,
            sync_tool: SyncAgentTimeTool::new(),
        }
    }

    pub async fn sync_with_server(&self, server: &str) -> Result<serde_json::Value, String> {
        self.sync_tool
            .execute(json!({
                "server": server,
                "protocol": "sntp"
            }))
            .await
            .map_err(|e| e.to_string())
    }
}

pub struct PerformanceMetrics {
    pub operation_name: String,
    pub total_operations: usize,
    pub successful_operations: usize,
    pub failed_operations: usize,
    pub min_latency_ms: f64,
    pub max_latency_ms: f64,
    pub avg_latency_ms: f64,
    pub p50_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub p99_latency_ms: f64,
}

impl PerformanceMetrics {
    pub fn new(operation_name: String) -> Self {
        Self {
            operation_name,
            total_operations: 0,
            successful_operations: 0,
            failed_operations: 0,
            min_latency_ms: f64::MAX,
            max_latency_ms: 0.0,
            avg_latency_ms: 0.0,
            p50_latency_ms: 0.0,
            p95_latency_ms: 0.0,
            p99_latency_ms: 0.0,
        }
    }

    pub fn from_latencies(operation_name: String, mut latencies: Vec<f64>) -> Self {
        let total = latencies.len();
        let successful = latencies.iter().filter(|&&l| l > 0.0).count();
        let failed = total - successful;

        if latencies.is_empty() {
            return Self {
                operation_name,
                total_operations: total,
                successful_operations: successful,
                failed_operations: failed,
                min_latency_ms: 0.0,
                max_latency_ms: 0.0,
                avg_latency_ms: 0.0,
                p50_latency_ms: 0.0,
                p95_latency_ms: 0.0,
                p99_latency_ms: 0.0,
            };
        }

        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let min = *latencies.first().unwrap();
        let max = *latencies.last().unwrap();
        let avg = latencies.iter().sum::<f64>() / latencies.len() as f64;

        let p50 = percentile(&latencies, 0.50);
        let p95 = percentile(&latencies, 0.95);
        let p99 = percentile(&latencies, 0.99);

        Self {
            operation_name,
            total_operations: total,
            successful_operations: successful,
            failed_operations: failed,
            min_latency_ms: min,
            max_latency_ms: max,
            avg_latency_ms: avg,
            p50_latency_ms: p50,
            p95_latency_ms: p95,
            p99_latency_ms: p99,
        }
    }

    pub fn print_report(&self) {
        println!("\n=== Performance Report: {} ===", self.operation_name);
        println!("Total operations: {}", self.total_operations);
        println!("  Successful: {}", self.successful_operations);
        println!("  Failed: {}", self.failed_operations);
        println!("Latency (ms):");
        println!("  Min: {:.2}", self.min_latency_ms);
        println!("  Max: {:.2}", self.max_latency_ms);
        println!("  Avg: {:.2}", self.avg_latency_ms);
        println!("  P50: {:.2}", self.p50_latency_ms);
        println!("  P95: {:.2}", self.p95_latency_ms);
        println!("  P99: {:.2}", self.p99_latency_ms);
    }
}

fn percentile(sorted_values: &[f64], p: f64) -> f64 {
    let idx = (p * (sorted_values.len() - 1) as f64).round() as usize;
    sorted_values[idx]
}

pub async fn run_concurrent_operations<F, Fut>(
    count: usize,
    max_concurrency: usize,
    operation: F,
) -> Vec<Result<f64, String>>
where
    F: Fn(usize) -> Fut,
    Fut: std::future::Future<Output = Result<f64, String>> + Send + 'static,
{
    let semaphore = Arc::new(Semaphore::new(max_concurrency));
    let mut handles = Vec::new();

    for i in 0..count {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let fut = operation(i);

        let handle = tokio::spawn(async move {
            let result = fut.await;
            drop(permit);
            result
        });

        handles.push(handle);
    }

    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => results.push(Err(format!("Task panicked: {e}"))),
        }
    }

    results
}

pub fn calculate_drift_statistics(offsets_ms: &[f64]) -> (f64, f64, f64) {
    if offsets_ms.is_empty() {
        return (0.0, 0.0, 0.0);
    }

    let mean = offsets_ms.iter().sum::<f64>() / offsets_ms.len() as f64;

    let variance =
        offsets_ms.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / offsets_ms.len() as f64;

    let std_dev = variance.sqrt();

    let max_drift = offsets_ms.iter().map(|&x| x.abs()).fold(0.0f64, f64::max);

    (mean, std_dev, max_drift)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentile() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((percentile(&values, 0.5) - 3.0).abs() < f64::EPSILON);
        assert!((percentile(&values, 0.0) - 1.0).abs() < f64::EPSILON);
        assert!((percentile(&values, 1.0) - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_metrics_from_latencies() {
        let latencies = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        let metrics = PerformanceMetrics::from_latencies("test".to_string(), latencies);

        assert_eq!(metrics.total_operations, 5);
        assert_eq!(metrics.successful_operations, 5);
        assert!((metrics.min_latency_ms - 10.0).abs() < f64::EPSILON);
        assert!((metrics.max_latency_ms - 50.0).abs() < f64::EPSILON);
        assert!((metrics.avg_latency_ms - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_drift_statistics() {
        let offsets = vec![-10.0, -5.0, 0.0, 5.0, 10.0];
        let (mean, std_dev, max_drift) = calculate_drift_statistics(&offsets);

        assert!(mean.abs() < f64::EPSILON);
        assert!(std_dev > 0.0);
        assert!((max_drift - 10.0).abs() < f64::EPSILON);
    }
}
