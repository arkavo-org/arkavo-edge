//! Learning velocity metrics for measuring improvement rate
//!
//! Tracks how fast the learning system improves over time,
//! enabling before/after comparisons across phases.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

const DEFAULT_WINDOW: usize = 100;

/// Snapshot of learning velocity at a point in time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningVelocity {
    /// Positive = improving, negative = degrading
    pub success_rate_trend: f64,
    /// Lower = better
    pub mean_retries_per_success: f64,
    /// Lower = more efficient
    pub cost_per_accepted_outcome: f64,
    /// Higher = learning faster
    pub lessons_per_hour: f64,
    /// How fast uncertainty decreases (higher = converging)
    pub convergence_rate: f64,
    /// When this snapshot was computed
    pub timestamp: DateTime<Utc>,
}

/// Tracks velocity signals over a sliding window
pub struct VelocityTracker {
    quality_scores: VecDeque<(DateTime<Utc>, f64)>,
    uncertainty_history: VecDeque<(DateTime<Utc>, f64)>,
    lesson_timestamps: VecDeque<DateTime<Utc>>,
    retry_counts: VecDeque<(DateTime<Utc>, u32)>,
    cost_history: VecDeque<(DateTime<Utc>, f64)>,
    window: usize,
}

impl Default for VelocityTracker {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW)
    }
}

impl VelocityTracker {
    /// Create a tracker with a given sliding window size
    pub fn new(window: usize) -> Self {
        let w = window.max(2);
        Self {
            quality_scores: VecDeque::with_capacity(w),
            uncertainty_history: VecDeque::with_capacity(w),
            lesson_timestamps: VecDeque::with_capacity(w),
            retry_counts: VecDeque::with_capacity(w),
            cost_history: VecDeque::with_capacity(w),
            window: w,
        }
    }

    /// Record a quality score (0.0-1.0) for a completed task
    pub fn record_quality(&mut self, score: f64) {
        let now = Utc::now();
        self.quality_scores.push_back((now, score));
        while self.quality_scores.len() > self.window {
            self.quality_scores.pop_front();
        }
    }

    /// Record model uncertainty (std_dev from BetaPrior)
    pub fn record_uncertainty(&mut self, std_dev: f64) {
        let now = Utc::now();
        self.uncertainty_history.push_back((now, std_dev));
        while self.uncertainty_history.len() > self.window {
            self.uncertainty_history.pop_front();
        }
    }

    /// Record a lesson synthesis event
    pub fn record_lesson(&mut self) {
        let now = Utc::now();
        self.lesson_timestamps.push_back(now);
        while self.lesson_timestamps.len() > self.window {
            self.lesson_timestamps.pop_front();
        }
    }

    /// Record retry count for a task attempt
    pub fn record_retries(&mut self, retries: u32) {
        let now = Utc::now();
        self.retry_counts.push_back((now, retries));
        while self.retry_counts.len() > self.window {
            self.retry_counts.pop_front();
        }
    }

    /// Record cost for an outcome
    pub fn record_cost(&mut self, cost_usd: f64) {
        let now = Utc::now();
        self.cost_history.push_back((now, cost_usd));
        while self.cost_history.len() > self.window {
            self.cost_history.pop_front();
        }
    }

    /// Compute current velocity snapshot
    pub fn compute(&self) -> LearningVelocity {
        LearningVelocity {
            success_rate_trend: self.compute_trend(),
            mean_retries_per_success: self.compute_mean_retries(),
            cost_per_accepted_outcome: self.compute_cost_per_outcome(),
            lessons_per_hour: self.compute_lesson_rate(),
            convergence_rate: self.compute_convergence(),
            timestamp: Utc::now(),
        }
    }

    /// Linear regression slope on quality scores (positive = improving)
    fn compute_trend(&self) -> f64 {
        if self.quality_scores.len() < 2 {
            return 0.0;
        }
        linear_slope(self.quality_scores.iter().map(|(_, v)| *v))
    }

    /// Average retries across the window
    fn compute_mean_retries(&self) -> f64 {
        if self.retry_counts.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.retry_counts.iter().map(|(_, r)| f64::from(*r)).sum();
        sum / self.retry_counts.len() as f64
    }

    /// Average cost per accepted outcome
    fn compute_cost_per_outcome(&self) -> f64 {
        if self.cost_history.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.cost_history.iter().map(|(_, c)| *c).sum();
        sum / self.cost_history.len() as f64
    }

    /// Lessons per hour based on timestamps
    fn compute_lesson_rate(&self) -> f64 {
        if self.lesson_timestamps.len() < 2 {
            return 0.0;
        }
        let first = self.lesson_timestamps.front().unwrap();
        let last = self.lesson_timestamps.back().unwrap();
        let hours = (*last - *first).num_seconds() as f64 / 3600.0;
        if hours < 0.001 {
            return 0.0;
        }
        (self.lesson_timestamps.len() - 1) as f64 / hours
    }

    /// Slope of uncertainty over time (negative = converging)
    fn compute_convergence(&self) -> f64 {
        if self.uncertainty_history.len() < 2 {
            return 0.0;
        }
        // Negative slope on uncertainty means converging, return as positive convergence
        -linear_slope(self.uncertainty_history.iter().map(|(_, v)| *v))
    }
}

/// Compute least-squares linear regression slope from an ordered sequence
fn linear_slope(values: impl Iterator<Item = f64>) -> f64 {
    let points: Vec<f64> = values.collect();
    let n = points.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xy = 0.0;
    let mut sum_x2 = 0.0;

    for (i, y) in points.iter().enumerate() {
        let x = i as f64;
        sum_x += x;
        sum_y += y;
        sum_xy = x.mul_add(*y, sum_xy);
        sum_x2 = x.mul_add(x, sum_x2);
    }

    let denom = n.mul_add(sum_x2, -(sum_x * sum_x));
    if denom.abs() < f64::EPSILON {
        return 0.0;
    }
    n.mul_add(sum_xy, -(sum_x * sum_y)) / denom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linear_slope_increasing() {
        let slope = linear_slope([1.0, 2.0, 3.0, 4.0, 5.0].into_iter());
        assert!((slope - 1.0).abs() < 0.01, "slope={slope}");
    }

    #[test]
    fn test_linear_slope_decreasing() {
        let slope = linear_slope([5.0, 4.0, 3.0, 2.0, 1.0].into_iter());
        assert!((slope - (-1.0)).abs() < 0.01, "slope={slope}");
    }

    #[test]
    fn test_linear_slope_flat() {
        let slope = linear_slope([0.5, 0.5, 0.5].into_iter());
        assert!(slope.abs() < 0.01, "slope={slope}");
    }

    #[test]
    fn test_linear_slope_insufficient_data() {
        assert_eq!(linear_slope([1.0].into_iter()), 0.0);
        assert_eq!(linear_slope(std::iter::empty()), 0.0);
    }

    #[test]
    fn test_velocity_tracker_quality_trend() {
        let mut tracker = VelocityTracker::new(10);
        for i in 0..5 {
            tracker.record_quality(i as f64 * 0.2);
        }
        let v = tracker.compute();
        assert!(v.success_rate_trend > 0.0, "should show improvement");
    }

    #[test]
    fn test_velocity_tracker_degrading() {
        let mut tracker = VelocityTracker::new(10);
        for i in (0..5).rev() {
            tracker.record_quality(i as f64 * 0.2);
        }
        let v = tracker.compute();
        assert!(v.success_rate_trend < 0.0, "should show degradation");
    }

    #[test]
    fn test_velocity_tracker_retries() {
        let mut tracker = VelocityTracker::new(10);
        tracker.record_retries(2);
        tracker.record_retries(3);
        tracker.record_retries(1);
        let v = tracker.compute();
        assert!((v.mean_retries_per_success - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_velocity_tracker_cost() {
        let mut tracker = VelocityTracker::new(10);
        tracker.record_cost(0.01);
        tracker.record_cost(0.03);
        let v = tracker.compute();
        assert!((v.cost_per_accepted_outcome - 0.02).abs() < 0.01);
    }

    #[test]
    fn test_velocity_tracker_convergence() {
        let mut tracker = VelocityTracker::new(10);
        // Uncertainty decreasing = converging
        tracker.record_uncertainty(0.4);
        tracker.record_uncertainty(0.3);
        tracker.record_uncertainty(0.2);
        tracker.record_uncertainty(0.1);
        let v = tracker.compute();
        assert!(
            v.convergence_rate > 0.0,
            "decreasing uncertainty should be positive convergence: {}",
            v.convergence_rate
        );
    }

    #[test]
    fn test_velocity_tracker_empty() {
        let tracker = VelocityTracker::new(10);
        let v = tracker.compute();
        assert_eq!(v.success_rate_trend, 0.0);
        assert_eq!(v.mean_retries_per_success, 0.0);
        assert_eq!(v.lessons_per_hour, 0.0);
        assert_eq!(v.convergence_rate, 0.0);
    }

    #[test]
    fn test_velocity_tracker_window_eviction() {
        let mut tracker = VelocityTracker::new(3);
        tracker.record_quality(0.1);
        tracker.record_quality(0.2);
        tracker.record_quality(0.3);
        tracker.record_quality(0.9); // evicts 0.1
        assert_eq!(tracker.quality_scores.len(), 3);
        // Should now contain 0.2, 0.3, 0.9
        let v = tracker.compute();
        assert!(v.success_rate_trend > 0.0);
    }
}
