//! Background scheduler for CPU-budgeted boundary probing
//!
//! Runs probing tasks during idle cycles with configurable CPU budget
//! to avoid impacting foreground operations.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::time::sleep;
use torg_core::Graph;

use crate::boundary::{BoundaryProbe, find_epsilon_boundary};
use crate::error::SatResult;

/// Default CPU budget (5%)
pub const DEFAULT_CPU_BUDGET: f64 = 0.05;

/// Default minimum interval between probe batches
pub const DEFAULT_MIN_INTERVAL: Duration = Duration::from_millis(100);

/// Default batch size
pub const DEFAULT_BATCH_SIZE: usize = 1;

/// Configuration for the background scheduler
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// CPU budget as percentage (0.0 - 1.0)
    pub cpu_budget: f64,
    /// Minimum delay between probe batches
    pub min_interval: Duration,
    /// Maximum probes per batch
    pub batch_size: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            cpu_budget: DEFAULT_CPU_BUDGET,
            min_interval: DEFAULT_MIN_INTERVAL,
            batch_size: DEFAULT_BATCH_SIZE,
        }
    }
}

/// Task to be probed in background
#[derive(Debug, Clone)]
pub struct ProbeTask {
    /// Graph to probe (wrapped in Arc for thread safety)
    pub graph: Arc<Graph>,
    /// Output node ID to probe
    pub output_id: u16,
    /// Epsilon distance for boundary probing
    pub epsilon: u32,
    /// Priority (higher = more urgent)
    pub priority: f64,
}

/// Result from a probe task
#[derive(Debug, Clone)]
pub struct ProbeResult {
    /// The task that was probed
    pub task: ProbeTask,
    /// Probes found (empty if error)
    pub probes: Vec<BoundaryProbe>,
    /// Error if probing failed
    pub error: Option<String>,
    /// Time taken for this probe
    pub duration: Duration,
}

/// Statistics from background probing
#[derive(Debug, Default)]
pub struct SchedulerStats {
    /// Number of probes completed
    pub probes_completed: AtomicU64,
    /// Total time spent probing (microseconds)
    pub total_time_us: AtomicU64,
    /// Number of anomalies (boundary probes) found
    pub anomalies_found: AtomicU64,
}

impl SchedulerStats {
    /// Get completed probe count
    pub fn completed(&self) -> u64 {
        self.probes_completed.load(Ordering::Relaxed)
    }

    /// Get total probing time
    pub fn total_time(&self) -> Duration {
        Duration::from_micros(self.total_time_us.load(Ordering::Relaxed))
    }

    /// Get anomaly count
    pub fn anomalies(&self) -> u64 {
        self.anomalies_found.load(Ordering::Relaxed)
    }

    /// Record a completed probe
    fn record(&self, duration: Duration, anomaly_count: usize) {
        self.probes_completed.fetch_add(1, Ordering::Relaxed);
        self.total_time_us
            .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
        self.anomalies_found
            .fetch_add(anomaly_count as u64, Ordering::Relaxed);
    }
}

/// Handle to control background probing
pub struct ProbeScheduler {
    config: SchedulerConfig,
    cancel_token: Arc<AtomicBool>,
    stats: Arc<SchedulerStats>,
}

impl ProbeScheduler {
    /// Create a new scheduler with default configuration
    pub fn new() -> Self {
        Self::with_config(SchedulerConfig::default())
    }

    /// Create a new scheduler with custom configuration
    pub fn with_config(config: SchedulerConfig) -> Self {
        Self {
            config,
            cancel_token: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(SchedulerStats::default()),
        }
    }

    /// Get the cancel token for external cancellation
    pub fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_token)
    }

    /// Request cancellation of background probing
    pub fn cancel(&self) {
        self.cancel_token.store(true, Ordering::SeqCst);
    }

    /// Check if scheduler is cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.load(Ordering::SeqCst)
    }

    /// Get current statistics
    pub fn stats(&self) -> &SchedulerStats {
        &self.stats
    }

    /// Start background probing with task queue
    ///
    /// Consumes tasks from `task_rx` and sends results to `result_tx`.
    /// Respects CPU budget by sleeping between batches.
    pub async fn start(
        &self,
        mut task_rx: mpsc::Receiver<ProbeTask>,
        result_tx: mpsc::Sender<ProbeResult>,
    ) {
        while !self.is_cancelled() {
            // Wait for next task
            let task = match task_rx.recv().await {
                Some(t) => t,
                None => break, // Channel closed
            };

            if self.is_cancelled() {
                break;
            }

            // Process the task
            let start = Instant::now();
            let (probes, error) = self.probe_task(&task);
            let duration = start.elapsed();

            // Update stats
            self.stats.record(duration, probes.len());

            // Send result
            let result = ProbeResult {
                task,
                probes,
                error,
                duration,
            };

            if result_tx.send(result).await.is_err() {
                break; // Result channel closed
            }

            // Enforce CPU budget
            let sleep_duration = self.compute_sleep_duration(duration);
            if sleep_duration > Duration::ZERO {
                sleep(sleep_duration).await;
            }
        }
    }

    /// Process a single probe task
    fn probe_task(&self, task: &ProbeTask) -> (Vec<BoundaryProbe>, Option<String>) {
        match find_epsilon_boundary(&task.graph, task.output_id, task.epsilon) {
            Ok(probes) => (probes, None),
            Err(e) => (vec![], Some(e.to_string())),
        }
    }

    /// Compute sleep duration to maintain CPU budget
    ///
    /// If we want 5% CPU usage and worked for 5ms:
    /// sleep_time = work_time * (1.0 / budget - 1.0) = 5ms * 19 = 95ms
    fn compute_sleep_duration(&self, work_duration: Duration) -> Duration {
        if self.config.cpu_budget >= 1.0 {
            return Duration::ZERO;
        }

        let multiplier = (1.0 / self.config.cpu_budget) - 1.0;
        let sleep_us = (work_duration.as_micros() as f64 * multiplier) as u64;
        let computed = Duration::from_micros(sleep_us);

        // Ensure at least min_interval
        computed.max(self.config.min_interval)
    }
}

impl Default for ProbeScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Run a single probe task synchronously
///
/// Useful for testing or when async is not needed.
pub fn probe_sync(task: &ProbeTask) -> SatResult<Vec<BoundaryProbe>> {
    find_epsilon_boundary(&task.graph, task.output_id, task.epsilon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use torg_core::{Builder, Token};

    fn create_or_graph() -> Graph {
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn test_scheduler_config_default() {
        let config = SchedulerConfig::default();
        assert!((config.cpu_budget - 0.05).abs() < 0.001);
        assert_eq!(config.min_interval, Duration::from_millis(100));
        assert_eq!(config.batch_size, 1);
    }

    #[test]
    fn test_compute_sleep_duration() {
        let config = SchedulerConfig {
            cpu_budget: 0.05, // 5%
            min_interval: Duration::from_millis(10),
            batch_size: 1,
        };
        let scheduler = ProbeScheduler::with_config(config);

        // 5ms work with 5% budget = 95ms sleep
        let work = Duration::from_millis(5);
        let sleep = scheduler.compute_sleep_duration(work);

        // Should be at least 95ms (19x multiplier)
        assert!(sleep >= Duration::from_millis(95));
    }

    #[test]
    fn test_compute_sleep_respects_min_interval() {
        let config = SchedulerConfig {
            cpu_budget: 0.5, // 50%
            min_interval: Duration::from_millis(100),
            batch_size: 1,
        };
        let scheduler = ProbeScheduler::with_config(config);

        // Very short work
        let work = Duration::from_micros(100);
        let sleep = scheduler.compute_sleep_duration(work);

        // Should be at least min_interval
        assert!(sleep >= Duration::from_millis(100));
    }

    #[test]
    fn test_cancel() {
        let scheduler = ProbeScheduler::new();

        assert!(!scheduler.is_cancelled());

        scheduler.cancel();

        assert!(scheduler.is_cancelled());
    }

    #[test]
    fn test_stats() {
        let scheduler = ProbeScheduler::new();

        assert_eq!(scheduler.stats().completed(), 0);
        assert_eq!(scheduler.stats().anomalies(), 0);

        scheduler.stats.record(Duration::from_millis(10), 2);

        assert_eq!(scheduler.stats().completed(), 1);
        assert_eq!(scheduler.stats().anomalies(), 2);
        assert!(scheduler.stats().total_time() >= Duration::from_millis(10));
    }

    #[test]
    fn test_probe_sync() {
        let graph = Arc::new(create_or_graph());
        let task = ProbeTask {
            graph,
            output_id: 2,
            epsilon: 1,
            priority: 1.0,
        };

        let probes = probe_sync(&task).unwrap();
        assert!(!probes.is_empty());
    }

    #[tokio::test]
    async fn test_scheduler_processes_tasks() {
        let scheduler = ProbeScheduler::with_config(SchedulerConfig {
            cpu_budget: 1.0, // No throttling for test
            min_interval: Duration::ZERO,
            batch_size: 1,
        });

        let (task_tx, task_rx) = mpsc::channel(10);
        let (result_tx, mut result_rx) = mpsc::channel(10);

        // Spawn scheduler
        let cancel_token = scheduler.cancel_token();
        let scheduler_handle = tokio::spawn(async move {
            scheduler.start(task_rx, result_tx).await;
        });

        // Send a task
        let graph = Arc::new(create_or_graph());
        let task = ProbeTask {
            graph,
            output_id: 2,
            epsilon: 1,
            priority: 1.0,
        };
        task_tx.send(task).await.unwrap();

        // Receive result
        let result = result_rx.recv().await.unwrap();
        assert!(result.error.is_none());
        assert!(!result.probes.is_empty());

        // Cancel and cleanup
        cancel_token.store(true, Ordering::SeqCst);
        drop(task_tx);
        let _ = scheduler_handle.await;
    }

    #[tokio::test]
    async fn test_scheduler_respects_cancellation() {
        let scheduler = ProbeScheduler::new();

        let (task_tx, task_rx) = mpsc::channel(10);
        let (result_tx, mut result_rx) = mpsc::channel(10);

        // Pre-cancel
        scheduler.cancel();

        let scheduler_handle = tokio::spawn(async move {
            scheduler.start(task_rx, result_tx).await;
        });

        // Send a task - should not be processed
        let graph = Arc::new(create_or_graph());
        let task = ProbeTask {
            graph,
            output_id: 2,
            epsilon: 1,
            priority: 1.0,
        };
        let _ = task_tx.send(task).await;

        // Close channel
        drop(task_tx);

        // Wait for scheduler to finish
        let _ = scheduler_handle.await;

        // Should have no results
        assert!(result_rx.try_recv().is_err());
    }
}
