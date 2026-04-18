//! Experiment configuration and results for autoresearch sweeps

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// A single experiment configuration point in the search space
#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct ExperimentConfig {
    /// Whether thinking/CoT is enabled
    pub enable_thinking: bool,
    /// Temperature index into the search space's temperature array
    pub temperature_idx: usize,
    /// Max tokens index into the search space's max_tokens array
    pub max_tokens_idx: usize,
    /// Top-p index into the search space's top_p array
    pub top_p_idx: usize,
    /// Kernel variant (0=baseline, future: precompiled MSL variants)
    pub kernel_variant: u32,
}

impl fmt::Display for ExperimentConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "thinking={} temp_idx={} max_tok_idx={} top_p_idx={} kv={}",
            self.enable_thinking,
            self.temperature_idx,
            self.max_tokens_idx,
            self.top_p_idx,
            self.kernel_variant,
        )
    }
}

/// Result of a single experiment run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentResult {
    /// The configuration that was tested
    pub config: ExperimentConfig,
    /// Seed used for this experiment
    pub seed: u32,
    /// Tokens per second during generation (wall-clock)
    pub tok_per_sec: f64,
    /// Time to first token in milliseconds
    pub ttft_ms: f64,
    /// Total tokens generated
    pub tokens_generated: u32,
    /// Quality score from evaluator (0.0-1.0)
    pub quality_score: f64,
    /// Composite metric: quality / tokens
    pub quality_per_token: f64,
    /// Weighted quality: quality_per_token × ln(1 + tok/s)
    pub weighted_quality: f64,
    /// Wall-clock time for this experiment
    pub wall_time: Duration,
    /// Prompt category used
    pub prompt_category: String,
}

impl ExperimentResult {
    /// Compute weighted quality from raw metrics
    pub fn compute_weighted(quality_per_token: f64, tok_per_sec: f64) -> f64 {
        quality_per_token * tok_per_sec.ln_1p()
    }
}

/// Search space definition for parameter sweeps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSpace {
    /// Available temperature values
    pub temperatures: Vec<f32>,
    /// Available max_tokens values
    pub max_tokens: Vec<u32>,
    /// Available top_p values
    pub top_p_values: Vec<f32>,
    /// Number of kernel variants available
    pub kernel_variants: u32,
    /// Seeds for non-deterministic evaluation
    pub seeds: Vec<u32>,
    /// Per-experiment wall-time cap
    pub experiment_timeout: Duration,
    /// Total sweep budget
    pub total_budget: Duration,
    /// Maximum rounds (Thompson Sampling iterations)
    pub max_rounds: u32,
}

impl Default for SearchSpace {
    fn default() -> Self {
        Self {
            temperatures: vec![0.1, 0.3, 0.5, 0.7, 0.9, 1.2],
            max_tokens: vec![32, 64, 128, 256],
            top_p_values: vec![0.8, 0.9, 0.95, 1.0],
            kernel_variants: 1,
            seeds: vec![42, 137, 2718, 31415, 65537],
            experiment_timeout: Duration::from_secs(10),
            total_budget: Duration::from_mins(5),
            max_rounds: 100,
        }
    }
}

impl SearchSpace {
    /// Total number of unique configurations
    pub fn config_count(&self) -> usize {
        2 * self.temperatures.len()
            * self.max_tokens.len()
            * self.top_p_values.len()
            * self.kernel_variants as usize
    }

    /// Generate all configurations in the search space
    pub fn all_configs(&self) -> Vec<ExperimentConfig> {
        let mut configs = Vec::with_capacity(self.config_count());
        for thinking in [false, true] {
            for t_idx in 0..self.temperatures.len() {
                for m_idx in 0..self.max_tokens.len() {
                    for p_idx in 0..self.top_p_values.len() {
                        for kv in 0..self.kernel_variants {
                            configs.push(ExperimentConfig {
                                enable_thinking: thinking,
                                temperature_idx: t_idx,
                                max_tokens_idx: m_idx,
                                top_p_idx: p_idx,
                                kernel_variant: kv,
                            });
                        }
                    }
                }
            }
        }
        configs
    }

    /// Resolve a config's actual parameter values.
    /// Returns `None` if any index is out of bounds (e.g. from deserialized gossip data).
    pub fn resolve(&self, config: &ExperimentConfig) -> Option<(f32, u32, f32)> {
        Some((
            *self.temperatures.get(config.temperature_idx)?,
            *self.max_tokens.get(config.max_tokens_idx)?,
            *self.top_p_values.get(config.top_p_idx)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_search_space_size() {
        let space = SearchSpace::default();
        assert_eq!(space.config_count(), (2 * 6 * 4 * 4));
    }

    #[test]
    fn test_all_configs_matches_count() {
        let space = SearchSpace::default();
        assert_eq!(space.all_configs().len(), space.config_count());
    }

    #[test]
    fn test_resolve_config() {
        let space = SearchSpace::default();
        let config = ExperimentConfig {
            enable_thinking: false,
            temperature_idx: 2,
            max_tokens_idx: 1,
            top_p_idx: 3,
            kernel_variant: 0,
        };
        let (temp, max_tok, top_p) = space.resolve(&config).unwrap();
        assert!((temp - 0.5).abs() < f32::EPSILON);
        assert_eq!(max_tok, 64);
        assert!((top_p - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_weighted_quality_computation() {
        let wq = ExperimentResult::compute_weighted(0.01, 200.0);
        assert!(wq > 0.0);
        let wq_zero = ExperimentResult::compute_weighted(0.0, 200.0);
        assert!(wq_zero.abs() < f64::EPSILON);
    }

    #[test]
    fn test_config_display() {
        let config = ExperimentConfig {
            enable_thinking: true,
            temperature_idx: 0,
            max_tokens_idx: 0,
            top_p_idx: 0,
            kernel_variant: 0,
        };
        let s = format!("{config}");
        assert!(s.contains("thinking=true"));
    }

    #[test]
    fn test_config_serde_roundtrip() {
        let config = ExperimentConfig {
            enable_thinking: false,
            temperature_idx: 3,
            max_tokens_idx: 2,
            top_p_idx: 1,
            kernel_variant: 0,
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: ExperimentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, back);
    }
}
