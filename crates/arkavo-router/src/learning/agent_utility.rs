//! Agent utility types for Thompson Sampling

use chrono::{DateTime, Utc};
use rand::Rng;
use rand_distr::{Beta, Distribution};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;

/// Beta distribution parameters for Thompson Sampling
///
/// Represents uncertainty about an agent's success probability.
/// Higher alpha = more successes, higher beta = more failures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BetaPrior {
    /// Successes + prior (alpha parameter)
    pub alpha: f64,
    /// Failures + prior (beta parameter)
    pub beta: f64,
    /// When these priors were last updated
    pub last_updated: DateTime<Utc>,
}

impl Default for BetaPrior {
    fn default() -> Self {
        Self::cold_start()
    }
}

impl BetaPrior {
    /// Cold start: optimistic but uncertain priors (Beta(2, 1))
    ///
    /// This gives a mean of 0.67 with high variance, encouraging
    /// exploration of new agents while being somewhat optimistic.
    pub fn cold_start() -> Self {
        Self {
            alpha: 2.0,
            beta: 1.0,
            last_updated: Utc::now(),
        }
    }

    /// Create with explicit parameters
    pub fn new(alpha: f64, beta: f64) -> Self {
        Self {
            alpha: alpha.max(0.1),
            beta: beta.max(0.1),
            last_updated: Utc::now(),
        }
    }

    /// Sample from the Beta distribution for Thompson Sampling
    ///
    /// # Panics
    ///
    /// This function does not panic - invalid parameters fall back to Beta(1, 1).
    pub fn sample<R: Rng>(&self, rng: &mut R) -> f64 {
        // SAFETY: Beta::new(1.0, 1.0) is guaranteed to succeed
        let dist = Beta::new(self.alpha, self.beta)
            .unwrap_or_else(|_| Beta::new(1.0, 1.0).expect("Beta(1,1) is always valid"));
        dist.sample(rng)
    }

    /// Sample using thread-local RNG
    pub fn sample_default(&self) -> f64 {
        self.sample(&mut rand::thread_rng())
    }

    /// Update with a success outcome
    pub fn record_success(&mut self) {
        self.alpha += 1.0;
        self.last_updated = Utc::now();
    }

    /// Update with a failure outcome
    pub fn record_failure(&mut self) {
        self.beta += 1.0;
        self.last_updated = Utc::now();
    }

    /// Apply a fractional update (for retrospective credit assignment)
    pub fn apply_fractional_update(&mut self, success_weight: f64) {
        if success_weight > 0.0 {
            self.alpha += success_weight;
        } else {
            self.beta += success_weight.abs();
        }
        self.last_updated = Utc::now();
    }

    /// Expected value (mean of Beta distribution)
    pub fn expected_value(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Variance of Beta distribution (uncertainty measure)
    pub fn variance(&self) -> f64 {
        let n = self.alpha + self.beta;
        (self.alpha * self.beta) / (n * n * (n + 1.0))
    }

    /// Standard deviation (square root of variance)
    pub fn std_dev(&self) -> f64 {
        self.variance().sqrt()
    }

    /// Total observations (alpha + beta - prior)
    pub fn total_observations(&self) -> f64 {
        // Subtract the cold start prior (2 + 1 = 3)
        (self.alpha + self.beta - 3.0).max(0.0)
    }

    /// Decay the prior towards cold start (for concept drift)
    pub fn decay(&mut self, factor: f64) {
        let cold = Self::cold_start();
        self.alpha = self.alpha.mul_add(factor, cold.alpha * (1.0 - factor));
        self.beta = self.beta.mul_add(factor, cold.beta * (1.0 - factor));
        self.last_updated = Utc::now();
    }
}

/// Per-agent utility tracking with sliding window for concept drift
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUtility {
    /// Agent identifier
    pub agent_id: String,
    /// Thompson Sampling prior (global)
    pub prior: BetaPrior,
    /// Total successes (all time)
    pub total_successes: u64,
    /// Total failures (all time)
    pub total_failures: u64,
    /// Windowed successes (for concept drift)
    pub window_successes: u64,
    /// Windowed failures (for concept drift)
    pub window_failures: u64,
    /// Windowed prior for fast concept drift adaptation
    pub window_prior: BetaPrior,
    /// Task category performance (category -> BetaPrior)
    pub category_priors: HashMap<String, BetaPrior>,
    /// When the agent was first seen
    pub created_at: DateTime<Utc>,
    /// Is this a probationary (new) agent?
    pub probationary: bool,
    /// Burst-level feedback history
    pub recent_bursts: VecDeque<BurstFeedback>,
    /// Total cost incurred by this agent
    pub total_cost_usd: f64,
    /// Total tokens used
    pub total_tokens: u64,
    /// Average latency in milliseconds
    pub avg_latency_ms: f64,
    /// Latency sample count (for running average)
    latency_samples: u64,
}

impl AgentUtility {
    /// Create a new agent utility tracker
    pub fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            prior: BetaPrior::cold_start(),
            total_successes: 0,
            total_failures: 0,
            window_successes: 0,
            window_failures: 0,
            window_prior: BetaPrior::cold_start(),
            category_priors: HashMap::new(),
            created_at: Utc::now(),
            probationary: true,
            recent_bursts: VecDeque::new(),
            total_cost_usd: 0.0,
            total_tokens: 0,
            avg_latency_ms: 0.0,
            latency_samples: 0,
        }
    }

    /// Record a burst outcome
    pub fn record_outcome(&mut self, feedback: &BurstFeedback, max_recent: usize) {
        if feedback.success {
            self.prior.record_success();
            self.window_prior.record_success();
            self.total_successes += 1;
            self.window_successes += 1;
        } else {
            self.prior.record_failure();
            self.window_prior.record_failure();
            self.total_failures += 1;
            self.window_failures += 1;
        }

        // Update category-specific prior
        let category_prior = self
            .category_priors
            .entry(feedback.task_category.clone())
            .or_insert_with(BetaPrior::cold_start);

        if feedback.success {
            category_prior.record_success();
        } else {
            category_prior.record_failure();
        }

        // Update latency (running average)
        self.latency_samples += 1;
        self.avg_latency_ms +=
            (feedback.latency_ms as f64 - self.avg_latency_ms) / self.latency_samples as f64;

        // Store feedback
        self.recent_bursts.push_back(feedback.clone());
        while self.recent_bursts.len() > max_recent {
            self.recent_bursts.pop_front();
        }
    }

    /// Record cost and token usage
    pub fn record_usage(&mut self, cost_usd: f64, tokens: u64) {
        self.total_cost_usd += cost_usd;
        self.total_tokens += tokens;
    }

    /// Get the prior for a specific category, or global if not available
    pub fn get_prior(&self, category: Option<&str>) -> &BetaPrior {
        category
            .and_then(|c| self.category_priors.get(c))
            .unwrap_or(&self.prior)
    }

    /// Sample for Thompson Sampling, optionally category-specific
    pub fn sample(&self, category: Option<&str>) -> f64 {
        self.get_prior(category).sample_default()
    }

    /// Sample blending global and windowed priors for concept drift adaptation
    ///
    /// Uses weighted average: `(1 - window_weight) * global + window_weight * windowed`
    /// The `window_weight` increases as windowed observations accumulate.
    pub fn sample_blended(&self, category: Option<&str>) -> f64 {
        let global_prior = self.get_prior(category);

        // Weight windowed prior by its observation count relative to a threshold
        let window_obs = (self.window_successes + self.window_failures) as f64;
        let window_weight = (window_obs / 20.0).min(0.5); // Max 50% weight at 20+ observations

        if window_weight < 0.01 {
            // Not enough windowed data, use global only
            return global_prior.sample_default();
        }

        let global_sample = global_prior.sample_default();
        let window_sample = self.window_prior.sample_default();

        global_sample.mul_add(1.0 - window_weight, window_sample * window_weight)
    }

    /// Total observations for this agent
    pub fn total_observations(&self) -> u64 {
        self.total_successes + self.total_failures
    }

    /// Check if agent should leave probation
    pub fn check_graduation(&mut self, min_observations: u64) {
        if self.probationary && self.total_observations() >= min_observations {
            self.probationary = false;
        }
    }

    /// Decay window counts and prior for concept drift adaptation
    pub fn decay_window(&mut self, factor: f64) {
        self.window_successes = (self.window_successes as f64 * factor) as u64;
        self.window_failures = (self.window_failures as f64 * factor) as u64;
        self.window_prior.decay(factor);
    }
}

/// Burst-level (immediate) feedback
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurstFeedback {
    /// Burst identifier
    pub burst_id: Uuid,
    /// When this feedback was recorded
    pub timestamp: DateTime<Utc>,
    /// Latency in milliseconds
    pub latency_ms: u64,
    /// Whether the burst succeeded
    pub success: bool,
    /// Optional quality score (0.0 to 1.0)
    pub quality_score: Option<f64>,
    /// Task category for category-specific learning
    pub task_category: String,
    /// Cost incurred
    pub cost_usd: f64,
    /// Tokens used
    pub tokens_used: u64,
}

impl BurstFeedback {
    /// Create a success feedback
    pub fn success(burst_id: Uuid, task_category: String, latency_ms: u64) -> Self {
        Self {
            burst_id,
            timestamp: Utc::now(),
            latency_ms,
            success: true,
            quality_score: None,
            task_category,
            cost_usd: 0.0,
            tokens_used: 0,
        }
    }

    /// Create a failure feedback
    pub fn failure(burst_id: Uuid, task_category: String, latency_ms: u64) -> Self {
        Self {
            burst_id,
            timestamp: Utc::now(),
            latency_ms,
            success: false,
            quality_score: None,
            task_category,
            cost_usd: 0.0,
            tokens_used: 0,
        }
    }

    /// Set quality score
    pub fn with_quality(mut self, score: f64) -> Self {
        self.quality_score = Some(score.clamp(0.0, 1.0));
        self
    }

    /// Set usage metrics
    pub fn with_usage(mut self, cost_usd: f64, tokens: u64) -> Self {
        self.cost_usd = cost_usd;
        self.tokens_used = tokens;
        self
    }
}

/// Task-level (retrospective) report for credit assignment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalTaskReport {
    /// Task identifier
    pub task_id: Uuid,
    /// When the task completed
    pub completed_at: DateTime<Utc>,
    /// Ordered list of agents that contributed
    pub agent_contributions: Vec<AgentContribution>,
    /// Overall task success (0.0 to 1.0)
    pub final_reward: f64,
    /// Optional quality assessment
    pub quality_metrics: Option<QualityMetrics>,
}

impl FinalTaskReport {
    /// Create a successful task report
    pub fn success(task_id: Uuid, contributions: Vec<AgentContribution>) -> Self {
        Self {
            task_id,
            completed_at: Utc::now(),
            agent_contributions: contributions,
            final_reward: 1.0,
            quality_metrics: None,
        }
    }

    /// Create a failed task report
    pub fn failure(task_id: Uuid, contributions: Vec<AgentContribution>) -> Self {
        Self {
            task_id,
            completed_at: Utc::now(),
            agent_contributions: contributions,
            final_reward: 0.0,
            quality_metrics: None,
        }
    }

    /// Set final reward
    pub fn with_reward(mut self, reward: f64) -> Self {
        self.final_reward = reward.clamp(0.0, 1.0);
        self
    }

    /// Set quality metrics
    pub fn with_quality(mut self, metrics: QualityMetrics) -> Self {
        self.quality_metrics = Some(metrics);
        self
    }
}

/// Contribution from a single agent to a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContribution {
    /// Agent identifier
    pub agent_id: String,
    /// Position in the task sequence (0 = first)
    pub position: usize,
    /// Immediate reward for this step
    pub immediate_reward: f64,
}

/// Quality metrics for a completed task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityMetrics {
    /// Correctness score (0.0 to 1.0)
    pub correctness: f64,
    /// Completeness score (0.0 to 1.0)
    pub completeness: f64,
    /// Efficiency score (0.0 to 1.0)
    pub efficiency: f64,
}

impl QualityMetrics {
    /// Create new quality metrics
    pub fn new(correctness: f64, completeness: f64, efficiency: f64) -> Self {
        Self {
            correctness: correctness.clamp(0.0, 1.0),
            completeness: completeness.clamp(0.0, 1.0),
            efficiency: efficiency.clamp(0.0, 1.0),
        }
    }

    /// Overall quality score (average of components)
    pub fn overall(&self) -> f64 {
        (self.correctness + self.completeness + self.efficiency) / 3.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_beta_prior_cold_start() {
        let prior = BetaPrior::cold_start();
        assert_eq!(prior.alpha, 2.0);
        assert_eq!(prior.beta, 1.0);
        // Mean should be 2/3 = 0.667
        assert!((prior.expected_value() - 0.667).abs() < 0.01);
    }

    #[test]
    fn test_beta_prior_updates() {
        let mut prior = BetaPrior::cold_start();

        prior.record_success();
        assert_eq!(prior.alpha, 3.0);
        assert_eq!(prior.beta, 1.0);

        prior.record_failure();
        assert_eq!(prior.alpha, 3.0);
        assert_eq!(prior.beta, 2.0);
    }

    #[test]
    fn test_beta_prior_sampling() {
        let prior = BetaPrior::new(10.0, 10.0);
        let mut rng = rand::thread_rng();

        // With equal alpha/beta, samples should be around 0.5
        let samples: Vec<f64> = (0..1000).map(|_| prior.sample(&mut rng)).collect();
        let mean: f64 = samples.iter().sum::<f64>() / samples.len() as f64;
        assert!((mean - 0.5).abs() < 0.1);
    }

    #[test]
    fn test_agent_utility_creation() {
        let utility = AgentUtility::new("test-agent".to_string());
        assert!(utility.probationary);
        assert_eq!(utility.total_successes, 0);
        assert_eq!(utility.total_failures, 0);
    }

    #[test]
    fn test_agent_utility_graduation() {
        let mut utility = AgentUtility::new("test-agent".to_string());
        assert!(utility.probationary);

        // Add 49 observations - still probationary
        for _ in 0..49 {
            utility.total_successes += 1;
        }
        utility.check_graduation(50);
        assert!(utility.probationary);

        // Add one more - should graduate
        utility.total_successes += 1;
        utility.check_graduation(50);
        assert!(!utility.probationary);
    }

    #[test]
    fn test_sample_blended_adapts_to_recent_failures() {
        let mut utility = AgentUtility::new("test".to_string());

        // Add 50 global successes (high performing historically)
        for _ in 0..50 {
            utility.prior.record_success();
            utility.total_successes += 1;
        }

        // Now add 20 windowed failures (recent poor performance)
        for _ in 0..20 {
            utility.window_prior.record_failure();
            utility.window_failures += 1;
        }

        // Blended sample should be lower than pure global sample
        let mut blended_sum = 0.0;
        let mut global_sum = 0.0;
        for _ in 0..100 {
            blended_sum += utility.sample_blended(None);
            global_sum += utility.prior.sample_default();
        }

        assert!(
            blended_sum / 100.0 < global_sum / 100.0,
            "Blended sample should reflect recent failures"
        );
    }
}
