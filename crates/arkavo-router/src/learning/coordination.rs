//! Coordination metrics for cross-swarm learning
//!
//! Tracks propagation delay, consensus strength, and learning efficiency
//! across a swarm of agents.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use uuid::Uuid;

/// Metrics for swarm coordination quality
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinationMetrics {
    /// Swarm identifier
    pub swarm_id: String,
    /// When these metrics were computed
    pub timestamp: DateTime<Utc>,

    // Propagation metrics
    /// Average time for lessons to reach peers (ms)
    pub propagation_delay_ms: MovingAverage,
    /// Ratio of swarm that received the lesson
    pub lesson_reach_ratio: f64,

    // Consensus metrics
    /// Average ratio of approving votes to total
    pub consensus_strength: f64,
    /// Average time to reach quorum (ms)
    pub quorum_time_ms: MovingAverage,
    /// Ratio of lessons rejected by peers
    pub rejection_rate: f64,

    // Efficiency metrics
    /// Ratio of lessons that improved outcomes
    pub lesson_application_rate: f64,
    /// Ratio of duplicate lessons filtered
    pub duplicate_lesson_rate: f64,
}

impl Default for CoordinationMetrics {
    fn default() -> Self {
        Self {
            swarm_id: String::new(),
            timestamp: Utc::now(),
            propagation_delay_ms: MovingAverage::new(20),
            lesson_reach_ratio: 0.0,
            consensus_strength: 0.0,
            quorum_time_ms: MovingAverage::new(20),
            rejection_rate: 0.0,
            lesson_application_rate: 0.0,
            duplicate_lesson_rate: 0.0,
        }
    }
}

impl CoordinationMetrics {
    /// Create new metrics for a swarm
    pub fn new(swarm_id: String) -> Self {
        Self {
            swarm_id,
            ..Default::default()
        }
    }
}

/// Moving average with configurable window size
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovingAverage {
    /// Current average value
    pub current: f64,
    /// Window size for the moving average
    pub window_size: usize,
    /// Recent samples
    samples: VecDeque<f64>,
}

impl Default for MovingAverage {
    fn default() -> Self {
        Self::new(20)
    }
}

impl MovingAverage {
    /// Create a new moving average with the given window size
    pub fn new(window_size: usize) -> Self {
        Self {
            current: 0.0,
            window_size: window_size.max(1),
            samples: VecDeque::with_capacity(window_size),
        }
    }

    /// Add a sample and update the average
    pub fn add(&mut self, sample: f64) {
        self.samples.push_back(sample);
        while self.samples.len() > self.window_size {
            self.samples.pop_front();
        }
        self.recompute();
    }

    fn recompute(&mut self) {
        if self.samples.is_empty() {
            self.current = 0.0;
        } else {
            self.current = self.samples.iter().sum::<f64>() / self.samples.len() as f64;
        }
    }

    /// Get the current average
    pub fn value(&self) -> f64 {
        self.current
    }

    /// Get the number of samples
    pub fn count(&self) -> usize {
        self.samples.len()
    }
}

/// Tracks votes for a lesson
#[derive(Debug, Clone, Default)]
pub struct VoteStats {
    /// Number of approve votes
    pub approves: u32,
    /// Number of reject votes
    pub rejects: u32,
    /// When voting started
    pub started_at: Option<Instant>,
    /// When quorum was reached
    pub quorum_at: Option<Instant>,
}

impl VoteStats {
    /// Record a vote
    pub fn record_vote(&mut self, approve: bool) {
        if self.started_at.is_none() {
            self.started_at = Some(Instant::now());
        }
        if approve {
            self.approves += 1;
        } else {
            self.rejects += 1;
        }
    }

    /// Total votes cast
    pub fn total(&self) -> u32 {
        self.approves + self.rejects
    }

    /// Approval ratio
    pub fn approval_ratio(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            0.0
        } else {
            f64::from(self.approves) / f64::from(total)
        }
    }

    /// Check if quorum reached (2/3 majority)
    pub fn has_quorum(&self, swarm_size: usize) -> bool {
        let threshold = (swarm_size * 2).div_ceil(3);
        self.total() as usize >= threshold
    }

    /// Mark quorum as reached
    pub fn mark_quorum(&mut self) {
        if self.quorum_at.is_none() {
            self.quorum_at = Some(Instant::now());
        }
    }

    /// Time to quorum in milliseconds
    pub fn quorum_time_ms(&self) -> Option<u64> {
        match (self.started_at, self.quorum_at) {
            (Some(start), Some(end)) => Some(end.duration_since(start).as_millis() as u64),
            _ => None,
        }
    }
}

/// Collects coordination metrics
pub struct MetricsCollector {
    /// Swarm identifier
    swarm_id: String,
    /// Swarm size for quorum calculation
    swarm_size: usize,
    /// Propagation timing for each lesson
    propagation_timer: HashMap<Uuid, Instant>,
    /// Reception count per lesson
    reception_count: HashMap<Uuid, u32>,
    /// Vote statistics per lesson
    consensus_tracker: HashMap<Uuid, VoteStats>,
    /// Application outcomes (lesson_id, improved)
    application_outcomes: VecDeque<(Uuid, bool)>,
    /// Maximum outcomes to track
    max_outcomes: usize,
    /// Duplicate lessons seen
    duplicates_seen: u64,
    /// Total lessons seen
    total_lessons: u64,
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new(swarm_id: String, swarm_size: usize) -> Self {
        Self {
            swarm_id,
            swarm_size: swarm_size.max(1),
            propagation_timer: HashMap::new(),
            reception_count: HashMap::new(),
            consensus_tracker: HashMap::new(),
            application_outcomes: VecDeque::new(),
            max_outcomes: 100,
            duplicates_seen: 0,
            total_lessons: 0,
        }
    }

    /// Record when a lesson was created
    pub fn on_lesson_created(&mut self, lesson_id: Uuid) {
        self.propagation_timer.insert(lesson_id, Instant::now());
        self.reception_count.insert(lesson_id, 0);
        self.total_lessons += 1;
    }

    /// Record when a lesson was received by a peer
    pub fn on_lesson_received(&mut self, lesson_id: Uuid, _from_peer: &str) {
        *self.reception_count.entry(lesson_id).or_insert(0) += 1;
    }

    /// Record a duplicate lesson
    pub fn on_duplicate_lesson(&mut self) {
        self.duplicates_seen += 1;
    }

    /// Record a vote on a lesson
    pub fn on_vote_received(&mut self, lesson_id: Uuid, approve: bool) {
        let stats = self.consensus_tracker.entry(lesson_id).or_default();
        stats.record_vote(approve);

        if stats.has_quorum(self.swarm_size) {
            stats.mark_quorum();
        }
    }

    /// Record when a lesson was applied and its outcome
    pub fn on_lesson_applied(&mut self, lesson_id: Uuid, improved: bool) {
        self.application_outcomes.push_back((lesson_id, improved));
        while self.application_outcomes.len() > self.max_outcomes {
            self.application_outcomes.pop_front();
        }
    }

    /// Compute current metrics
    pub fn compute_metrics(&self) -> CoordinationMetrics {
        let mut metrics = CoordinationMetrics::new(self.swarm_id.clone());

        // Propagation delay
        for (lesson_id, start) in &self.propagation_timer {
            let elapsed = start.elapsed().as_millis() as f64;
            metrics.propagation_delay_ms.add(elapsed);

            // Reach ratio
            if let Some(&count) = self.reception_count.get(lesson_id) {
                let ratio = f64::from(count) / self.swarm_size as f64;
                metrics.lesson_reach_ratio = f64::midpoint(metrics.lesson_reach_ratio, ratio);
            }
        }

        // Consensus metrics
        let mut total_strength = 0.0;
        let mut strength_count = 0;
        let mut rejections = 0u64;

        for stats in self.consensus_tracker.values() {
            total_strength += stats.approval_ratio();
            strength_count += 1;

            if stats.approval_ratio() < 0.67 && stats.total() > 0 {
                rejections += 1;
            }

            if let Some(time) = stats.quorum_time_ms() {
                metrics.quorum_time_ms.add(time as f64);
            }
        }

        if strength_count > 0 {
            metrics.consensus_strength = total_strength / strength_count as f64;
            metrics.rejection_rate = rejections as f64 / strength_count as f64;
        }

        // Application rate
        let improved_count = self
            .application_outcomes
            .iter()
            .filter(|(_, ok)| *ok)
            .count();
        if !self.application_outcomes.is_empty() {
            metrics.lesson_application_rate =
                improved_count as f64 / self.application_outcomes.len() as f64;
        }

        // Duplicate rate
        if self.total_lessons > 0 {
            metrics.duplicate_lesson_rate = self.duplicates_seen as f64 / self.total_lessons as f64;
        }

        metrics.timestamp = Utc::now();
        metrics
    }

    /// Reset metrics (useful after persisting)
    pub fn reset(&mut self) {
        self.propagation_timer.clear();
        self.reception_count.clear();
        self.consensus_tracker.clear();
        self.application_outcomes.clear();
        self.duplicates_seen = 0;
        self.total_lessons = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_moving_average() {
        let mut avg = MovingAverage::new(3);
        avg.add(10.0);
        assert!((avg.value() - 10.0).abs() < 0.01);

        avg.add(20.0);
        assert!((avg.value() - 15.0).abs() < 0.01);

        avg.add(30.0);
        assert!((avg.value() - 20.0).abs() < 0.01);

        // Window slides
        avg.add(40.0);
        assert!((avg.value() - 30.0).abs() < 0.01);
    }

    #[test]
    fn test_vote_stats() {
        let mut stats = VoteStats::default();
        stats.record_vote(true);
        stats.record_vote(true);
        stats.record_vote(false);

        assert_eq!(stats.approves, 2);
        assert_eq!(stats.rejects, 1);
        assert!((stats.approval_ratio() - 0.667).abs() < 0.01);
    }

    #[test]
    fn test_quorum_detection() {
        let mut stats = VoteStats::default();

        // 3-agent swarm needs 2 votes for quorum
        stats.record_vote(true);
        assert!(!stats.has_quorum(3));

        stats.record_vote(true);
        assert!(stats.has_quorum(3));
    }

    #[test]
    fn test_metrics_collector() {
        let mut collector = MetricsCollector::new("test-swarm".to_string(), 3);

        let lesson_id = Uuid::new_v4();
        collector.on_lesson_created(lesson_id);
        collector.on_lesson_received(lesson_id, "peer-1");
        collector.on_lesson_received(lesson_id, "peer-2");
        collector.on_vote_received(lesson_id, true);
        collector.on_vote_received(lesson_id, true);
        collector.on_lesson_applied(lesson_id, true);

        let metrics = collector.compute_metrics();
        assert!(metrics.consensus_strength > 0.9);
        assert!(metrics.lesson_application_rate > 0.9);
    }
}
