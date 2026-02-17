//! Regret tracking for counterfactual evaluation
//!
//! Implements cumulative regret calculation with outcome attribution windows.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use uuid::Uuid;

/// Default attribution window (1 second)
pub const DEFAULT_ATTRIBUTION_WINDOW: Duration = Duration::from_secs(1);

/// Default maximum outcomes to keep in memory
pub const DEFAULT_MAX_OUTCOMES: usize = 10_000;

/// Record of a single evaluation outcome
#[derive(Debug, Clone)]
pub struct Outcome {
    /// When the outcome occurred
    pub timestamp: Instant,
    /// Hash of the input for deduplication
    pub input_hash: u64,
    /// Cost of the production policy
    pub production_cost: f64,
    /// Costs of each candidate policy
    pub candidate_costs: HashMap<Uuid, f64>,
}

/// Per-candidate regret statistics using Welford's algorithm
#[derive(Debug, Clone)]
pub struct CandidateRegret {
    /// Candidate identifier
    pub candidate_id: Uuid,
    /// Cumulative regret (sum of production_cost - candidate_cost)
    pub cumulative_regret: f64,
    /// Number of outcomes observed
    pub outcome_count: u64,
    /// Running mean of regret values
    pub mean_regret: f64,
    /// Running M2 for variance calculation (sum of squared differences)
    m2: f64,
    /// When positive regret streak started
    pub positive_streak_start: Option<Instant>,
}

impl CandidateRegret {
    /// Create a new regret tracker for a candidate
    pub fn new(candidate_id: Uuid) -> Self {
        Self {
            candidate_id,
            cumulative_regret: 0.0,
            outcome_count: 0,
            mean_regret: 0.0,
            m2: 0.0,
            positive_streak_start: None,
        }
    }

    /// Update with a new regret observation using Welford's algorithm
    ///
    /// Regret = production_cost - candidate_cost
    /// Positive regret means the candidate is better than production
    pub fn update(&mut self, regret: f64) {
        self.cumulative_regret += regret;
        self.outcome_count += 1;

        // Welford's online algorithm for mean and variance
        let n = self.outcome_count as f64;
        let delta = regret - self.mean_regret;
        self.mean_regret += delta / n;
        let delta2 = regret - self.mean_regret;
        self.m2 += delta * delta2;

        // Track positive regret streak
        if regret > 0.0 {
            if self.positive_streak_start.is_none() {
                self.positive_streak_start = Some(Instant::now());
            }
        } else {
            self.positive_streak_start = None;
        }
    }

    /// Get the sample variance
    pub fn variance(&self) -> f64 {
        if self.outcome_count < 2 {
            0.0
        } else {
            self.m2 / (self.outcome_count - 1) as f64
        }
    }

    /// Get the sample standard deviation
    pub fn std_dev(&self) -> f64 {
        self.variance().sqrt()
    }

    /// Get hours of positive regret streak
    pub fn positive_streak_hours(&self) -> f64 {
        self.positive_streak_start
            .map(|start| start.elapsed().as_secs_f64() / 3600.0)
            .unwrap_or(0.0)
    }

    /// Check if candidate has sustained positive regret
    pub fn has_sustained_advantage(&self, hours: f64) -> bool {
        self.mean_regret > 0.0 && self.positive_streak_hours() >= hours
    }
}

/// Accumulates regret for all candidates
pub struct RegretAccumulator {
    /// Recent outcomes for attribution window
    outcomes: VecDeque<Outcome>,
    /// Per-candidate regret statistics
    candidate_regrets: HashMap<Uuid, CandidateRegret>,
    /// Attribution window duration
    attribution_window: Duration,
    /// Maximum outcomes to retain
    max_outcomes: usize,
}

impl RegretAccumulator {
    /// Create a new regret accumulator with default settings
    pub fn new() -> Self {
        Self::with_config(DEFAULT_ATTRIBUTION_WINDOW, DEFAULT_MAX_OUTCOMES)
    }

    /// Create with custom configuration
    pub fn with_config(attribution_window: Duration, max_outcomes: usize) -> Self {
        Self {
            outcomes: VecDeque::with_capacity(max_outcomes),
            candidate_regrets: HashMap::new(),
            attribution_window,
            max_outcomes,
        }
    }

    /// Record a new outcome
    pub fn record_outcome(&mut self, outcome: Outcome) {
        // Update regret for each candidate
        for (&candidate_id, &candidate_cost) in &outcome.candidate_costs {
            let regret = outcome.production_cost - candidate_cost;

            let entry = self
                .candidate_regrets
                .entry(candidate_id)
                .or_insert_with(|| CandidateRegret::new(candidate_id));

            entry.update(regret);
        }

        // Expire old outcomes
        self.expire_old_outcomes();

        // Add new outcome
        if self.outcomes.len() >= self.max_outcomes {
            self.outcomes.pop_front();
        }
        self.outcomes.push_back(outcome);
    }

    /// Expire outcomes outside the attribution window
    fn expire_old_outcomes(&mut self) {
        let now = Instant::now();
        // Use checked_sub to handle edge case where attribution_window exceeds elapsed time
        let cutoff = now.checked_sub(self.attribution_window);

        match cutoff {
            Some(cutoff) => {
                // Normal case: expire outcomes older than cutoff
                while let Some(oldest) = self.outcomes.front() {
                    if oldest.timestamp < cutoff {
                        self.outcomes.pop_front();
                    } else {
                        break;
                    }
                }
            }
            None => {
                // Edge case: attribution_window exceeds elapsed time
                // All outcomes are within the window, nothing to expire
            }
        }
    }

    /// Get regret statistics for a candidate
    pub fn get_regret(&self, candidate_id: Uuid) -> Option<&CandidateRegret> {
        self.candidate_regrets.get(&candidate_id)
    }

    /// Get mutable regret statistics for a candidate
    pub fn get_regret_mut(&mut self, candidate_id: Uuid) -> Option<&mut CandidateRegret> {
        self.candidate_regrets.get_mut(&candidate_id)
    }

    /// Remove a candidate from tracking
    pub fn remove_candidate(&mut self, candidate_id: Uuid) {
        self.candidate_regrets.remove(&candidate_id);
    }

    /// Get all tracked candidates
    pub fn candidates(&self) -> impl Iterator<Item = &CandidateRegret> {
        self.candidate_regrets.values()
    }

    /// Get number of outcomes in window
    pub fn outcome_count(&self) -> usize {
        self.outcomes.len()
    }

    /// Get recent outcomes
    pub fn recent_outcomes(&self) -> impl Iterator<Item = &Outcome> {
        self.outcomes.iter()
    }

    /// Reset positive streak for a candidate
    pub fn reset_streak(&mut self, candidate_id: Uuid) {
        if let Some(regret) = self.candidate_regrets.get_mut(&candidate_id) {
            regret.positive_streak_start = None;
        }
    }
}

impl Default for RegretAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash inputs for deduplication
pub fn hash_inputs(inputs: &HashMap<u16, bool>) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    let mut sorted: Vec<_> = inputs.iter().collect();
    sorted.sort_by_key(|(k, _)| *k);
    for (k, v) in sorted {
        k.hash(&mut hasher);
        v.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candidate_regret_update() {
        let id = Uuid::new_v4();
        let mut regret = CandidateRegret::new(id);

        // Add some observations
        regret.update(1.0);
        regret.update(2.0);
        regret.update(3.0);

        assert_eq!(regret.outcome_count, 3);
        assert!((regret.cumulative_regret - 6.0).abs() < f64::EPSILON);
        assert!((regret.mean_regret - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_welford_variance() {
        let id = Uuid::new_v4();
        let mut regret = CandidateRegret::new(id);

        // Known values: [2, 4, 6, 8, 10] mean=6, var=10
        for v in [2.0, 4.0, 6.0, 8.0, 10.0] {
            regret.update(v);
        }

        assert!((regret.mean_regret - 6.0).abs() < f64::EPSILON);
        assert!((regret.variance() - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_positive_streak() {
        let id = Uuid::new_v4();
        let mut regret = CandidateRegret::new(id);

        assert!(regret.positive_streak_start.is_none());

        regret.update(1.0);
        assert!(regret.positive_streak_start.is_some());

        regret.update(2.0);
        assert!(regret.positive_streak_start.is_some());

        regret.update(-1.0);
        assert!(regret.positive_streak_start.is_none());
    }

    #[test]
    fn test_regret_accumulator_basic() {
        let mut acc = RegretAccumulator::new();
        let candidate_id = Uuid::new_v4();

        let mut costs = HashMap::new();
        costs.insert(candidate_id, 0.5);

        acc.record_outcome(Outcome {
            timestamp: Instant::now(),
            input_hash: 12345,
            production_cost: 1.0,
            candidate_costs: costs,
        });

        let regret = acc.get_regret(candidate_id).unwrap();
        assert_eq!(regret.outcome_count, 1);
        assert!((regret.mean_regret - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_regret_accumulator_multiple_candidates() {
        let mut acc = RegretAccumulator::new();
        let c1 = Uuid::new_v4();
        let c2 = Uuid::new_v4();

        let mut costs = HashMap::new();
        costs.insert(c1, 0.3); // Better than production
        costs.insert(c2, 1.2); // Worse than production

        acc.record_outcome(Outcome {
            timestamp: Instant::now(),
            input_hash: 12345,
            production_cost: 1.0,
            candidate_costs: costs,
        });

        let r1 = acc.get_regret(c1).unwrap();
        assert!(r1.mean_regret > 0.0); // Positive regret = candidate is better

        let r2 = acc.get_regret(c2).unwrap();
        assert!(r2.mean_regret < 0.0); // Negative regret = candidate is worse
    }

    #[test]
    fn test_hash_inputs() {
        let mut inputs1 = HashMap::new();
        inputs1.insert(0, true);
        inputs1.insert(1, false);

        let mut inputs2 = HashMap::new();
        inputs2.insert(1, false);
        inputs2.insert(0, true);

        // Order shouldn't matter
        assert_eq!(hash_inputs(&inputs1), hash_inputs(&inputs2));

        // Different inputs should produce different hashes
        let mut inputs3 = HashMap::new();
        inputs3.insert(0, false);
        inputs3.insert(1, false);

        assert_ne!(hash_inputs(&inputs1), hash_inputs(&inputs3));
    }

    #[test]
    fn test_remove_candidate() {
        let mut acc = RegretAccumulator::new();
        let candidate_id = Uuid::new_v4();

        let mut costs = HashMap::new();
        costs.insert(candidate_id, 0.5);

        acc.record_outcome(Outcome {
            timestamp: Instant::now(),
            input_hash: 12345,
            production_cost: 1.0,
            candidate_costs: costs,
        });

        assert!(acc.get_regret(candidate_id).is_some());

        acc.remove_candidate(candidate_id);
        assert!(acc.get_regret(candidate_id).is_none());
    }
}
