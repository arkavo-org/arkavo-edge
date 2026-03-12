//! Context strategy optimization for the autoresearch subsystem
//!
//! Sweeps context strategy parameters using the same Thompson Sampling
//! infrastructure as inference parameters. The evaluation signal comes from
//! task completion metrics: loop detector success rate, burst completion
//! ratio, and context overhead (bytes passed vs bytes useful).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Context strategy configuration to sweep
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Which context strategy variant to use
    pub strategy: ContextVariant,
    /// Minimum context size (chars) before offloading to ledger
    pub min_offload_chars: usize,
    /// For LastNMessages: how many messages to keep
    pub last_n: usize,
}

/// Enumerated context strategy variants (mirrors ContextStrategy but hashable)
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextVariant {
    Full,
    SummaryOnly,
    LastNMessages,
    ArtifactReference,
    Ledger,
}

impl fmt::Display for ContextVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => write!(f, "full"),
            Self::SummaryOnly => write!(f, "summary_only"),
            Self::LastNMessages => write!(f, "last_n_messages"),
            Self::ArtifactReference => write!(f, "artifact_reference"),
            Self::Ledger => write!(f, "ledger"),
        }
    }
}

/// Metrics from a context strategy evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMetrics {
    /// Fraction of bursts that completed successfully (0.0-1.0)
    pub completion_rate: f64,
    /// Fraction of subtasks that avoided loop detection (0.0-1.0)
    pub loop_avoidance_rate: f64,
    /// Average context bytes passed per burst
    pub avg_context_bytes: usize,
    /// Average quality score of completed bursts
    pub avg_quality: f64,
    /// Number of bursts evaluated
    pub burst_count: u32,
}

impl ContextMetrics {
    /// Composite score: completion × loop_avoidance × quality, penalized by context size
    ///
    /// A strategy that completes 90% of bursts with no loops and low context overhead
    /// beats one that completes 95% but passes 10x the context bytes.
    pub fn composite_score(&self) -> f64 {
        let effectiveness = self.completion_rate * self.loop_avoidance_rate * self.avg_quality;
        // Penalize excessive context: log(1 + bytes/1000) normalization
        let overhead_penalty = 1.0 / (1.0 + self.avg_context_bytes as f64 / 10_000.0);
        effectiveness * overhead_penalty
    }
}

/// Search space for context strategy optimization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSearchSpace {
    /// All configurations to explore
    pub configs: Vec<ContextConfig>,
}

impl Default for ContextSearchSpace {
    fn default() -> Self {
        let mut configs = Vec::new();

        // Full strategy — no parameters to sweep
        configs.push(ContextConfig {
            strategy: ContextVariant::Full,
            min_offload_chars: 0,
            last_n: 0,
        });

        // SummaryOnly — no parameters
        configs.push(ContextConfig {
            strategy: ContextVariant::SummaryOnly,
            min_offload_chars: 0,
            last_n: 0,
        });

        // LastNMessages — sweep N
        for n in [3, 5, 10, 20] {
            configs.push(ContextConfig {
                strategy: ContextVariant::LastNMessages,
                min_offload_chars: 0,
                last_n: n,
            });
        }

        // ArtifactReference — no parameters
        configs.push(ContextConfig {
            strategy: ContextVariant::ArtifactReference,
            min_offload_chars: 0,
            last_n: 0,
        });

        // Ledger — sweep min_offload_chars
        for threshold in [500, 1000, 2000, 5000, 10000] {
            configs.push(ContextConfig {
                strategy: ContextVariant::Ledger,
                min_offload_chars: threshold,
                last_n: 0,
            });
        }

        Self { configs }
    }
}

impl ContextSearchSpace {
    /// Number of configurations in the search space
    pub fn config_count(&self) -> usize {
        self.configs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_search_space() {
        let space = ContextSearchSpace::default();
        // 1 Full + 1 SummaryOnly + 4 LastN + 1 ArtifactRef + 5 Ledger = 12
        assert_eq!(space.config_count(), 12);
    }

    #[test]
    fn test_context_metrics_composite_score() {
        let good = ContextMetrics {
            completion_rate: 0.95,
            loop_avoidance_rate: 1.0,
            avg_context_bytes: 500,
            avg_quality: 0.9,
            burst_count: 20,
        };
        let bad = ContextMetrics {
            completion_rate: 0.5,
            loop_avoidance_rate: 0.7,
            avg_context_bytes: 50_000,
            avg_quality: 0.6,
            burst_count: 20,
        };
        assert!(
            good.composite_score() > bad.composite_score(),
            "good={} > bad={}",
            good.composite_score(),
            bad.composite_score()
        );
    }

    #[test]
    fn test_composite_score_penalizes_large_context() {
        let small_ctx = ContextMetrics {
            completion_rate: 0.9,
            loop_avoidance_rate: 0.9,
            avg_context_bytes: 100,
            avg_quality: 0.8,
            burst_count: 10,
        };
        let large_ctx = ContextMetrics {
            completion_rate: 0.9,
            loop_avoidance_rate: 0.9,
            avg_context_bytes: 100_000,
            avg_quality: 0.8,
            burst_count: 10,
        };
        assert!(
            small_ctx.composite_score() > large_ctx.composite_score(),
            "small={} > large={}",
            small_ctx.composite_score(),
            large_ctx.composite_score()
        );
    }

    #[test]
    fn test_composite_score_zero_completion() {
        let zero = ContextMetrics {
            completion_rate: 0.0,
            loop_avoidance_rate: 1.0,
            avg_context_bytes: 100,
            avg_quality: 1.0,
            burst_count: 5,
        };
        assert!(zero.composite_score().abs() < f64::EPSILON);
    }

    #[test]
    fn test_context_variant_display() {
        assert_eq!(format!("{}", ContextVariant::Full), "full");
        assert_eq!(format!("{}", ContextVariant::Ledger), "ledger");
        assert_eq!(
            format!("{}", ContextVariant::LastNMessages),
            "last_n_messages"
        );
    }

    #[test]
    fn test_context_config_serde_roundtrip() {
        let config = ContextConfig {
            strategy: ContextVariant::Ledger,
            min_offload_chars: 2000,
            last_n: 0,
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: ContextConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, back);
    }

    #[test]
    fn test_last_n_variants_in_space() {
        let space = ContextSearchSpace::default();
        let last_n_configs: Vec<&ContextConfig> = space
            .configs
            .iter()
            .filter(|c| c.strategy == ContextVariant::LastNMessages)
            .collect();
        assert_eq!(last_n_configs.len(), 4);
        let n_values: Vec<usize> = last_n_configs.iter().map(|c| c.last_n).collect();
        assert!(n_values.contains(&3));
        assert!(n_values.contains(&20));
    }

    #[test]
    fn test_ledger_variants_in_space() {
        let space = ContextSearchSpace::default();
        let ledger_configs: Vec<&ContextConfig> = space
            .configs
            .iter()
            .filter(|c| c.strategy == ContextVariant::Ledger)
            .collect();
        assert_eq!(ledger_configs.len(), 5);
        let thresholds: Vec<usize> = ledger_configs.iter().map(|c| c.min_offload_chars).collect();
        assert!(thresholds.contains(&500));
        assert!(thresholds.contains(&10000));
    }
}
