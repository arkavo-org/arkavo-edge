//! Thompson Sampling for autoresearch experiment selection
//!
//! Beta distribution priors guide exploration: each configuration arm
//! maintains a BetaPrior that gets updated with fractional quality scores.
//! The sampler selects the arm with the highest Thompson sample each round.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Maximum plausible prior value (10,000 observations is an extremely strong prior)
const MAX_PRIOR: f64 = 10_000.0;

/// Validate that alpha and beta values are safe for use as Beta priors.
/// Rejects non-finite, negative, and implausibly large values.
fn validate_prior(alpha: f64, beta: f64) -> bool {
    alpha.is_finite()
        && beta.is_finite()
        && alpha >= 1.0
        && beta >= 1.0
        && alpha <= MAX_PRIOR
        && beta <= MAX_PRIOR
}

/// Beta distribution prior for a single arm
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BetaPrior {
    pub alpha: f64,
    pub beta: f64,
}

impl BetaPrior {
    /// Uniform prior: Beta(1, 1)
    pub fn new() -> Self {
        Self {
            alpha: 1.0,
            beta: 1.0,
        }
    }

    /// Sample from Beta(alpha, beta) — returns value in [0, 1]
    pub fn sample(&self, rng: &mut Rng) -> f64 {
        let x = gamma_sample(self.alpha, rng);
        let y = gamma_sample(self.beta, rng);
        if x + y > 0.0 { x / (x + y) } else { 0.5 }
    }

    /// Update with a fractional quality score (0.0 = failure, 1.0 = perfect)
    pub fn update(&mut self, quality: f64) {
        let clamped = quality.clamp(0.0, 1.0);
        self.alpha += clamped;
        self.beta += 1.0 - clamped;
    }

    /// Expected value: alpha / (alpha + beta)
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Number of observations: alpha + beta - 2 (subtracting the prior)
    pub fn observations(&self) -> f64 {
        self.alpha + self.beta - 2.0
    }

    /// Check whether this prior has plausible values (finite, bounded, non-negative)
    pub fn is_valid(&self) -> bool {
        validate_prior(self.alpha, self.beta)
    }
}

impl Default for BetaPrior {
    fn default() -> Self {
        Self::new()
    }
}

/// Thompson Sampler managing priors for all arms
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThompsonSampler {
    /// Per-arm priors indexed by arm ID
    pub priors: HashMap<usize, BetaPrior>,
    /// Total arms in the search space
    pub arm_count: usize,
}

impl ThompsonSampler {
    /// Create a sampler for `arm_count` arms with uniform priors
    pub fn new(arm_count: usize) -> Self {
        Self {
            priors: HashMap::new(),
            arm_count,
        }
    }

    /// Select the next arm to try via Thompson Sampling
    pub fn select(&self, rng: &mut Rng) -> usize {
        (0..self.arm_count)
            .map(|i| {
                let prior = self.priors.get(&i).cloned().unwrap_or_default();
                (i, prior.sample(rng))
            })
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    /// Update the prior for an arm with observed quality
    pub fn update(&mut self, arm: usize, quality: f64) {
        self.priors.entry(arm).or_default().update(quality);
    }

    /// Get the prior for an arm (uniform default if unseen)
    pub fn prior(&self, arm: usize) -> BetaPrior {
        self.priors.get(&arm).cloned().unwrap_or_default()
    }

    /// Best arm by posterior mean
    pub fn best_arm(&self) -> Option<usize> {
        self.priors
            .iter()
            .max_by(|a, b| {
                a.1.mean()
                    .partial_cmp(&b.1.mean())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(k, _)| *k)
    }

    /// Merge remote priors (from gossip) into local state.
    /// Rejects invalid priors (non-finite, negative, or implausibly large)
    /// and clamps merged values to prevent accumulation beyond bounds.
    pub fn merge_remote(&mut self, remote: &HashMap<usize, BetaPrior>) {
        for (arm, remote_prior) in remote {
            if !remote_prior.is_valid() {
                continue;
            }
            let local = self.priors.entry(*arm).or_default();
            // Additive merge: combine observations
            local.alpha += remote_prior.alpha - 1.0; // subtract uniform prior
            local.beta += remote_prior.beta - 1.0;
            // Clamp to prevent unbounded accumulation across many merges
            local.alpha = local.alpha.clamp(1.0, MAX_PRIOR);
            local.beta = local.beta.clamp(1.0, MAX_PRIOR);
        }
    }
}

/// Minimal xorshift64 RNG (no external dependency needed)
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    /// Seed from system time
    pub fn from_time() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(12345);
        Self::new(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Gamma(alpha, 1) sample via Marsaglia-Tsang
#[allow(clippy::min_ident_chars)]
fn gamma_sample(alpha: f64, rng: &mut Rng) -> f64 {
    if alpha < 1.0 {
        let uniform = rng.next_f64();
        return gamma_sample(alpha + 1.0, rng) * uniform.powf(1.0 / alpha);
    }
    let shift = alpha - 1.0 / 3.0;
    let scale = 1.0 / (9.0 * shift).sqrt();
    loop {
        let normal = normal_sample(rng);
        let candidate = (1.0 + scale * normal).powi(3);
        if candidate <= 0.0 {
            continue;
        }
        let uniform = rng.next_f64();
        if uniform < 0.0331f64.mul_add(-normal.powi(4), 1.0) {
            return shift * candidate;
        }
        if uniform.ln() < (0.5 * normal).mul_add(normal, shift * (1.0 - candidate + candidate.ln()))
        {
            return shift * candidate;
        }
    }
}

/// Box-Muller normal sample
fn normal_sample(rng: &mut Rng) -> f64 {
    let u1 = rng.next_f64().max(f64::MIN_POSITIVE);
    let u2 = rng.next_f64();
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_beta_prior_initial() {
        let p = BetaPrior::new();
        assert!((p.mean() - 0.5).abs() < f64::EPSILON);
        assert!(p.observations().abs() < f64::EPSILON);
    }

    #[test]
    fn test_beta_prior_update_success() {
        let mut p = BetaPrior::new();
        p.update(1.0);
        assert!((p.mean() - 2.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_beta_prior_update_failure() {
        let mut p = BetaPrior::new();
        p.update(0.0);
        assert!((p.mean() - 1.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_beta_prior_convergence() {
        let mut p = BetaPrior::new();
        for _ in 0..100 {
            p.update(0.8);
        }
        assert!((p.mean() - 0.8).abs() < 0.05, "mean={}", p.mean());
    }

    #[test]
    fn test_thompson_sampler_select() {
        let sampler = ThompsonSampler::new(10);
        let mut rng = Rng::new(42);
        let arm = sampler.select(&mut rng);
        assert!(arm < 10);
    }

    #[test]
    fn test_thompson_sampler_prefers_good_arm() {
        let mut sampler = ThompsonSampler::new(10);
        // Give arm 3 many successes
        for _ in 0..50 {
            sampler.update(3, 0.9);
        }
        // Give all others failures
        for arm in 0..10 {
            if arm != 3 {
                for _ in 0..20 {
                    sampler.update(arm, 0.1);
                }
            }
        }

        let mut rng = Rng::new(42);
        let mut count_3 = 0;
        for _ in 0..100 {
            if sampler.select(&mut rng) == 3 {
                count_3 += 1;
            }
        }
        assert!(count_3 > 30, "arm 3 selected {count_3}/100");
    }

    #[test]
    fn test_thompson_best_arm() {
        let mut sampler = ThompsonSampler::new(5);
        sampler.update(2, 0.9);
        sampler.update(2, 0.8);
        sampler.update(4, 0.1);
        assert_eq!(sampler.best_arm(), Some(2));
    }

    #[test]
    fn test_merge_remote_priors() {
        let mut local = ThompsonSampler::new(5);
        local.update(0, 0.7);

        let mut remote = HashMap::new();
        let mut remote_prior = BetaPrior::new();
        for _ in 0..10 {
            remote_prior.update(0.8);
        }
        remote.insert(0, remote_prior);

        let before = local.prior(0).mean();
        local.merge_remote(&remote);
        let after = local.prior(0).mean();
        // After merging 10 high-quality observations, mean should increase
        assert!(after > before, "before={before}, after={after}");
    }

    #[test]
    fn test_is_valid_boundary_cases() {
        assert!(
            BetaPrior {
                alpha: 1.0,
                beta: 1.0
            }
            .is_valid()
        );
        assert!(
            BetaPrior {
                alpha: 10_000.0,
                beta: 10_000.0
            }
            .is_valid()
        );
        assert!(
            !BetaPrior {
                alpha: 0.5,
                beta: 1.0
            }
            .is_valid()
        );
        assert!(
            !BetaPrior {
                alpha: 1.0,
                beta: 0.5
            }
            .is_valid()
        );
        assert!(
            !BetaPrior {
                alpha: 10_001.0,
                beta: 1.0
            }
            .is_valid()
        );
        assert!(
            !BetaPrior {
                alpha: f64::NAN,
                beta: 1.0
            }
            .is_valid()
        );
        assert!(
            !BetaPrior {
                alpha: f64::INFINITY,
                beta: 1.0
            }
            .is_valid()
        );
        assert!(
            !BetaPrior {
                alpha: f64::NEG_INFINITY,
                beta: 1.0
            }
            .is_valid()
        );
        assert!(
            !BetaPrior {
                alpha: -1.0,
                beta: 1.0
            }
            .is_valid()
        );
    }

    #[test]
    fn test_merge_remote_rejects_nan() {
        let mut sampler = ThompsonSampler::new(5);
        sampler.update(0, 0.7);
        let before = sampler.prior(0).mean();

        let mut remote = HashMap::new();
        remote.insert(
            0,
            BetaPrior {
                alpha: f64::NAN,
                beta: 1.0,
            },
        );
        sampler.merge_remote(&remote);

        let after = sampler.prior(0).mean();
        assert!(
            (after - before).abs() < f64::EPSILON,
            "NaN prior should be skipped"
        );
    }

    #[test]
    fn test_merge_remote_rejects_infinity() {
        let mut sampler = ThompsonSampler::new(5);
        sampler.update(0, 0.7);
        let before = sampler.prior(0).mean();

        let mut remote = HashMap::new();
        remote.insert(
            0,
            BetaPrior {
                alpha: f64::INFINITY,
                beta: 1.0,
            },
        );
        sampler.merge_remote(&remote);

        let after = sampler.prior(0).mean();
        assert!(
            (after - before).abs() < f64::EPSILON,
            "infinite prior should be skipped"
        );
    }

    #[test]
    fn test_merge_remote_rejects_negative() {
        let mut sampler = ThompsonSampler::new(5);
        sampler.update(0, 0.7);
        let before = sampler.prior(0).mean();

        let mut remote = HashMap::new();
        remote.insert(
            0,
            BetaPrior {
                alpha: -5.0,
                beta: 1.0,
            },
        );
        sampler.merge_remote(&remote);

        let after = sampler.prior(0).mean();
        assert!(
            (after - before).abs() < f64::EPSILON,
            "negative prior should be skipped"
        );
    }

    #[test]
    fn test_merge_remote_clamps_accumulated() {
        let mut sampler = ThompsonSampler::new(5);
        // Merge many valid large priors to test accumulation clamping
        for _ in 0..20 {
            let mut remote = HashMap::new();
            remote.insert(
                0,
                BetaPrior {
                    alpha: 9000.0,
                    beta: 1.0,
                },
            );
            sampler.merge_remote(&remote);
        }
        let prior = sampler.prior(0);
        assert!(
            prior.alpha <= 10_000.0,
            "alpha={} should be clamped",
            prior.alpha
        );
        assert!(
            prior.beta >= 1.0,
            "beta={} should be at least 1.0",
            prior.beta
        );
    }

    #[test]
    fn test_rng_deterministic() {
        let mut r1 = Rng::new(42);
        let mut r2 = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(r1.next_u64(), r2.next_u64());
        }
    }

    #[test]
    fn test_rng_range() {
        let mut rng = Rng::new(42);
        for _ in 0..1000 {
            let v = rng.next_f64();
            assert!((0.0..1.0).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn test_sampler_serde_roundtrip() {
        let mut sampler = ThompsonSampler::new(3);
        sampler.update(0, 0.7);
        sampler.update(1, 0.3);
        let json = serde_json::to_string(&sampler).unwrap();
        let back: ThompsonSampler = serde_json::from_str(&json).unwrap();
        assert_eq!(back.arm_count, 3);
        assert!((back.prior(0).mean() - sampler.prior(0).mean()).abs() < f64::EPSILON);
    }
}
