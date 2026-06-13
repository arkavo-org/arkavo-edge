//! Runtime-updateable optimal inference configs per model.
//!
//! Seeded from compile-time defaults (`ModelChoice::optimal_sampling()`),
//! updated via autoresearch sweeps and gossip propagation.

use crate::decision::ModelChoice;
use arkavo_llm::ThinkingMode;
use std::collections::HashMap;
use std::sync::RwLock;

/// Optimal sampling configuration discovered via autoresearch
#[derive(Debug, Clone)]
pub struct OptimalConfig {
    pub temperature: f32,
    pub top_p: f32,
    pub thinking_mode: ThinkingMode,
    /// Confidence from Thompson posterior mean (0.0 = uniform prior, 1.0 = converged)
    pub confidence: f64,
}

/// Thread-safe store for per-model optimal inference configs.
/// Seeded from compile-time defaults, updated via autoresearch sweeps and gossip.
pub struct OptimalConfigStore {
    configs: RwLock<HashMap<String, OptimalConfig>>,
}

impl OptimalConfigStore {
    /// Seed from `ModelChoice::optimal_sampling()` compile-time defaults
    #[must_use]
    pub fn new() -> Self {
        let mut configs = HashMap::new();
        for model in ModelChoice::ALL_LOCAL {
            if let Some((temp, top_p, thinking)) = model.optimal_sampling() {
                configs.insert(
                    model.name().to_string(),
                    OptimalConfig {
                        temperature: temp,
                        top_p,
                        thinking_mode: thinking,
                        confidence: 1.0,
                    },
                );
            }
        }
        Self {
            configs: RwLock::new(configs),
        }
    }

    /// Read the optimal config for a model (clone under read lock)
    pub fn get(&self, model: &ModelChoice) -> Option<OptimalConfig> {
        self.configs
            .read()
            .ok()
            .and_then(|c| c.get(model.name()).cloned())
    }

    /// Update config only if new confidence >= existing (prevents gossip downgrade)
    pub fn update(&self, model: &ModelChoice, config: OptimalConfig) {
        if let Ok(mut configs) = self.configs.write() {
            let dominated = configs
                .get(model.name())
                .is_some_and(|existing| existing.confidence > config.confidence);
            if !dominated {
                configs.insert(model.name().to_string(), config);
            }
        }
    }

    /// Snapshot all configs for telemetry (clone under read lock)
    pub fn snapshot(&self) -> Vec<(String, OptimalConfig)> {
        self.configs
            .read()
            .ok()
            .map(|c| c.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }

    /// Convenience wrapper for sweep results
    pub fn update_from_sweep(
        &self,
        model: &ModelChoice,
        temperature: f32,
        top_p: f32,
        thinking_mode: ThinkingMode,
        posterior_mean: f64,
    ) {
        self.update(
            model,
            OptimalConfig {
                temperature,
                top_p,
                thinking_mode,
                confidence: posterior_mean,
            },
        );
    }
}

impl Default for OptimalConfigStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("ROUTER-014")]
    #[test]
    fn seeded_from_compile_time_defaults() {
        let store = OptimalConfigStore::new();
        let config = store.get(&ModelChoice::LocalQwen3);
        assert!(config.is_some());
        let c = config.unwrap();
        assert!((c.confidence - 1.0).abs() < f64::EPSILON);
    }

    #[spec("ROUTER-014")]
    #[test]
    fn update_respects_confidence() {
        let store = OptimalConfigStore::new();
        let model = ModelChoice::LocalQwen3;

        // Lower confidence should not overwrite seeded config (confidence 1.0)
        store.update_from_sweep(&model, 0.5, 0.5, ThinkingMode::On, 0.3);
        let c = store.get(&model).unwrap();
        assert!((c.temperature - 0.9).abs() < f32::EPSILON); // unchanged

        // Equal confidence should update
        store.update_from_sweep(&model, 0.5, 0.5, ThinkingMode::On, 1.0);
        let c = store.get(&model).unwrap();
        assert!((c.temperature - 0.5).abs() < f32::EPSILON); // updated
    }

    #[spec("ROUTER-014")]
    #[test]
    fn get_returns_none_for_cloud_models() {
        let store = OptimalConfigStore::new();
        assert!(store.get(&ModelChoice::ClaudeSonnet).is_none());
    }
}
