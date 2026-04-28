//! Adaptation engine for Agent Runtime Policy (§6).
//!
//! Implements Thompson Sampling, epsilon-greedy, UCB1, and static
//! entity selection with Beta distribution priors, cold start policy,
//! and version-bound prior reset.

use std::collections::HashMap;

use arkavo_arp::adaptation::{Adaptation, AdaptationMethod, VersionBinding};
use arkavo_arp::model::BetaPrior;
use rand::Rng;
use rand_distr::{Beta, Distribution};

/// An entity that can be selected by the adaptation engine (model, tool, peer).
#[derive(Debug, Clone)]
pub struct Entity {
    pub id: String,
    pub version_hash: Option<String>,
    pub available: bool,
}

/// Observed outcome for updating priors.
#[derive(Debug, Clone, Copy)]
pub struct Outcome {
    pub success: bool,
    pub quality_score: f64,
}

/// The adaptation engine selects entities using configured method (§6).
pub struct AdaptationEngine {
    config: Adaptation,
    priors: HashMap<String, BetaPrior>,
    observation_counts: HashMap<String, u32>,
    version_hashes: HashMap<String, String>,
    total_selections: u64,
}

impl AdaptationEngine {
    /// Create an engine from ARP adaptation config.
    pub fn new(config: Adaptation) -> Self {
        Self {
            config,
            priors: HashMap::new(),
            observation_counts: HashMap::new(),
            version_hashes: HashMap::new(),
            total_selections: 0,
        }
    }

    /// Select the best entity from a feasible set.
    /// Returns `None` if no entities are available.
    pub fn select(&mut self, entities: &[Entity]) -> Option<String> {
        let available: Vec<&Entity> = entities.iter().filter(|e| e.available).collect();
        if available.is_empty() {
            return None;
        }

        self.check_version_resets(&available);

        let selected = match self.config.method {
            AdaptationMethod::ThompsonSampling => self.thompson_select(&available),
            AdaptationMethod::EpsilonGreedy => self.epsilon_greedy_select(&available),
            AdaptationMethod::Ucb1 => self.ucb1_select(&available),
            AdaptationMethod::Static => Some(available[0].id.clone()),
        };

        if let Some(ref id) = selected {
            self.total_selections += 1;
            *self.observation_counts.entry(id.clone()).or_insert(0) += 1;
        }

        selected
    }

    /// Update the prior for an entity based on observed outcome.
    pub fn update(&mut self, entity_id: &str, outcome: Outcome) {
        let prior = self.prior_for(entity_id);
        if outcome.success {
            prior.alpha += outcome.quality_score;
        } else {
            prior.beta += 1.0 - outcome.quality_score;
        }
    }

    /// Get the current prior for an entity.
    pub fn get_prior(&self, entity_id: &str) -> BetaPrior {
        self.priors
            .get(entity_id)
            .copied()
            .unwrap_or_else(|| self.initial_prior())
    }

    /// Get observation count for an entity.
    pub fn observations(&self, entity_id: &str) -> u32 {
        self.observation_counts.get(entity_id).copied().unwrap_or(0)
    }

    /// Whether an entity is still in warmup (insufficient observations).
    pub fn in_warmup(&self, entity_id: &str) -> bool {
        let warmup_period = self
            .config
            .cold_start
            .as_ref()
            .and_then(|c| c.warmup_period)
            .unwrap_or(5);
        self.observations(entity_id) < warmup_period
    }

    /// Reset the prior for a specific entity to initial state.
    pub fn reset_prior(&mut self, entity_id: &str) {
        let initial = self.reset_state();
        self.priors.insert(entity_id.to_string(), initial);
        self.observation_counts.insert(entity_id.to_string(), 0);
    }

    /// Snapshot of all known entity priors plus their observation counts.
    /// Used by UIs and audit tooling to inspect engine state.
    pub fn snapshot(&self) -> Vec<EntityPriorSnapshot> {
        let mut ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for k in self.priors.keys() {
            ids.insert(k.as_str());
        }
        for k in self.observation_counts.keys() {
            ids.insert(k.as_str());
        }
        let warmup_period = self
            .config
            .cold_start
            .as_ref()
            .and_then(|c| c.warmup_period)
            .unwrap_or(5);
        let mut out: Vec<EntityPriorSnapshot> = ids
            .into_iter()
            .map(|id| {
                let prior = self.get_prior(id);
                let observations = self.observations(id);
                let mean = prior.alpha / (prior.alpha + prior.beta).max(f64::EPSILON);
                EntityPriorSnapshot {
                    id: id.to_string(),
                    alpha: prior.alpha,
                    beta: prior.beta,
                    mean,
                    observations,
                    in_warmup: observations < warmup_period,
                }
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// The configured adaptation method for this engine.
    pub fn method(&self) -> AdaptationMethod {
        self.config.method
    }

    fn initial_prior(&self) -> BetaPrior {
        self.config
            .cold_start
            .as_ref()
            .and_then(|c| c.initial_prior)
            .unwrap_or_default()
    }

    fn reset_state(&self) -> BetaPrior {
        self.config
            .prior_management
            .as_ref()
            .and_then(|pm| pm.reset_state)
            .unwrap_or_else(|| self.initial_prior())
    }

    fn prior_for(&mut self, entity_id: &str) -> &mut BetaPrior {
        let initial = self.initial_prior();
        self.priors.entry(entity_id.to_string()).or_insert(initial)
    }

    /// Thompson Sampling: draw a sample from Beta(alpha, beta) for each entity,
    /// pick the one with the highest sample. This provides natural
    /// exploration/exploitation balance — uncertain entities have high-variance
    /// draws that occasionally beat well-known leaders.
    fn thompson_select(&mut self, entities: &[&Entity]) -> Option<String> {
        let exploration_floor = self
            .config
            .parameters
            .as_ref()
            .and_then(|p| p.exploration_floor)
            .unwrap_or(0.05);

        let mut rng = rand::thread_rng();
        let mut best_id = None;
        let mut best_score = f64::NEG_INFINITY;

        for entity in entities {
            let prior = self.get_prior(&entity.id).sanitize();
            let dist = Beta::new(prior.alpha, prior.beta).unwrap_or_else(|_| {
                // Fallback to uniform Beta(1,1) if parameters are somehow invalid
                Beta::new(1.0, 1.0).unwrap()
            });
            let sample = dist.sample(&mut rng) + exploration_floor;

            if sample > best_score {
                best_score = sample;
                best_id = Some(entity.id.clone());
            }
        }

        best_id
    }

    /// Epsilon-greedy: with probability epsilon, pick uniformly at random;
    /// otherwise pick the entity with the highest mean.
    fn epsilon_greedy_select(&mut self, entities: &[&Entity]) -> Option<String> {
        let epsilon = self
            .config
            .parameters
            .as_ref()
            .and_then(|p| p.epsilon)
            .unwrap_or(0.1);

        let mut rng = rand::thread_rng();

        if rng.r#gen::<f64>() < epsilon {
            // Explore: uniform random selection
            let idx = rng.r#gen_range(0..entities.len());
            return Some(entities[idx].id.clone());
        }

        // Exploit: pick entity with highest mean
        self.pick_best_mean(entities)
    }

    /// UCB1: pick entity maximizing mean + sqrt(2 * ln(total) / count).
    fn ucb1_select(&mut self, entities: &[&Entity]) -> Option<String> {
        let total = self.total_selections.max(1) as f64;

        let mut best_id = None;
        let mut best_score = f64::NEG_INFINITY;

        for entity in entities {
            let prior = self.get_prior(&entity.id);
            let mean = prior.alpha / (prior.alpha + prior.beta);
            let count = self.observations(&entity.id).max(1) as f64;
            let ucb = mean + (2.0 * total.ln() / count).sqrt();

            if ucb > best_score {
                best_score = ucb;
                best_id = Some(entity.id.clone());
            }
        }

        best_id
    }

    fn pick_best_mean(&self, entities: &[&Entity]) -> Option<String> {
        entities
            .iter()
            .max_by(|a, b| {
                let pa = self.get_prior(&a.id);
                let pb = self.get_prior(&b.id);
                let ma = pa.alpha / (pa.alpha + pa.beta);
                let mb = pb.alpha / (pb.alpha + pb.beta);
                ma.partial_cmp(&mb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|e| e.id.clone())
    }

    /// Check if any entity's version hash changed, and reset priors if configured.
    fn check_version_resets(&mut self, entities: &[&Entity]) {
        let binding = self
            .config
            .prior_management
            .as_ref()
            .and_then(|pm| pm.version_binding)
            .unwrap_or(VersionBinding::EntityHash);

        let should_reset = self
            .config
            .prior_management
            .as_ref()
            .and_then(|pm| pm.reset_on_version_change)
            .unwrap_or(true);

        if !should_reset || binding == VersionBinding::NameOnly {
            return;
        }

        for entity in entities {
            if let Some(new_hash) = &entity.version_hash {
                if let Some(old_hash) = self.version_hashes.get(&entity.id)
                    && old_hash != new_hash
                {
                    self.reset_prior(&entity.id);
                }
                self.version_hashes
                    .insert(entity.id.clone(), new_hash.clone());
            }
        }
    }
}

/// Read-only snapshot of an entity's prior plus observation count.
#[derive(Debug, Clone)]
pub struct EntityPriorSnapshot {
    pub id: String,
    pub alpha: f64,
    pub beta: f64,
    pub mean: f64,
    pub observations: u32,
    pub in_warmup: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_arp::adaptation::{
        AdaptationParameters, ColdStart, ColdStartStrategy, PriorManagement, WarmupBehavior,
    };

    fn entities() -> Vec<Entity> {
        vec![
            Entity {
                id: "model_a".into(),
                version_hash: Some("v1".into()),
                available: true,
            },
            Entity {
                id: "model_b".into(),
                version_hash: Some("v1".into()),
                available: true,
            },
            Entity {
                id: "model_c".into(),
                version_hash: Some("v1".into()),
                available: false,
            },
        ]
    }

    fn thompson_config() -> Adaptation {
        Adaptation {
            method: AdaptationMethod::ThompsonSampling,
            parameters: Some(AdaptationParameters {
                exploration_floor: Some(0.05),
                epsilon: None,
                epsilon_decay: None,
                epsilon_min: None,
            }),
            cold_start: Some(ColdStart {
                strategy: Some(ColdStartStrategy::Optimistic),
                initial_prior: Some(BetaPrior {
                    alpha: 2.0,
                    beta: 1.0,
                }),
                warmup_period: Some(5),
                warmup_behavior: Some(WarmupBehavior::ConstitutionalOnly),
            }),
            prior_management: Some(PriorManagement {
                version_binding: Some(VersionBinding::EntityHash),
                reset_on_version_change: Some(true),
                reset_state: Some(BetaPrior {
                    alpha: 2.0,
                    beta: 1.0,
                }),
            }),
            signal_separation: None,
        }
    }

    #[test]
    fn thompson_sampling_selects_available_entity() {
        let mut engine = AdaptationEngine::new(thompson_config());
        // Run multiple times since Thompson Sampling is stochastic
        for _ in 0..10 {
            let result = engine.select(&entities());
            assert!(result.is_some());
            let id = result.unwrap();
            assert!(id == "model_a" || id == "model_b");
            // model_c is unavailable, should never be selected
            assert_ne!(id, "model_c");
        }
    }

    #[test]
    fn no_available_entities_returns_none() {
        let mut engine = AdaptationEngine::new(thompson_config());
        let unavailable = vec![Entity {
            id: "x".into(),
            version_hash: None,
            available: false,
        }];
        assert!(engine.select(&unavailable).is_none());
    }

    #[test]
    fn update_shifts_prior() {
        let mut engine = AdaptationEngine::new(thompson_config());
        let prior_before = engine.get_prior("model_a");

        engine.update(
            "model_a",
            Outcome {
                success: true,
                quality_score: 0.9,
            },
        );

        let prior_after = engine.get_prior("model_a");
        assert!(prior_after.alpha > prior_before.alpha);
        assert_eq!(prior_after.beta, prior_before.beta);
    }

    #[test]
    fn failure_increases_beta() {
        let mut engine = AdaptationEngine::new(thompson_config());
        let prior_before = engine.get_prior("model_a");

        engine.update(
            "model_a",
            Outcome {
                success: false,
                quality_score: 0.2,
            },
        );

        let prior_after = engine.get_prior("model_a");
        assert_eq!(prior_after.alpha, prior_before.alpha);
        assert!(prior_after.beta > prior_before.beta);
    }

    #[test]
    fn convergence_to_better_entity() {
        let mut engine = AdaptationEngine::new(thompson_config());

        // model_a succeeds consistently, model_b fails
        for _ in 0..20 {
            engine.update(
                "model_a",
                Outcome {
                    success: true,
                    quality_score: 0.9,
                },
            );
            engine.update(
                "model_b",
                Outcome {
                    success: false,
                    quality_score: 0.1,
                },
            );
        }

        // After enough evidence, should strongly prefer model_a
        let prior_a = engine.get_prior("model_a");
        let prior_b = engine.get_prior("model_b");
        let mean_a = prior_a.alpha / (prior_a.alpha + prior_a.beta);
        let mean_b = prior_b.alpha / (prior_b.alpha + prior_b.beta);
        assert!(mean_a > mean_b);

        // With strong evidence, model_a should win most selections
        let mut a_count = 0;
        for _ in 0..100 {
            if engine.select(&entities()).unwrap() == "model_a" {
                a_count += 1;
            }
        }
        assert!(
            a_count > 80,
            "model_a selected {a_count}/100 times, expected >80"
        );
    }

    #[test]
    fn version_change_resets_prior() {
        let mut engine = AdaptationEngine::new(thompson_config());
        let ents = entities();

        // Build up a strong prior for model_a
        engine.select(&ents);
        for _ in 0..10 {
            engine.update(
                "model_a",
                Outcome {
                    success: true,
                    quality_score: 0.95,
                },
            );
        }
        let prior_before = engine.get_prior("model_a");
        assert!(prior_before.alpha > 5.0);

        // Simulate version change
        let updated = vec![
            Entity {
                id: "model_a".into(),
                version_hash: Some("v2".into()),
                available: true,
            },
            Entity {
                id: "model_b".into(),
                version_hash: Some("v1".into()),
                available: true,
            },
        ];
        engine.select(&updated);

        let prior_after = engine.get_prior("model_a");
        assert_eq!(prior_after.alpha, 2.0);
        assert_eq!(prior_after.beta, 1.0);
    }

    #[test]
    fn warmup_detection() {
        let mut engine = AdaptationEngine::new(thompson_config());
        assert!(engine.in_warmup("new_entity"));

        for _ in 0..5 {
            engine.select(&[Entity {
                id: "new_entity".into(),
                version_hash: None,
                available: true,
            }]);
        }
        assert!(!engine.in_warmup("new_entity"));
    }

    #[test]
    fn epsilon_greedy_method() {
        let config = Adaptation {
            method: AdaptationMethod::EpsilonGreedy,
            parameters: Some(AdaptationParameters {
                exploration_floor: None,
                epsilon: Some(0.1),
                epsilon_decay: None,
                epsilon_min: None,
            }),
            cold_start: None,
            prior_management: None,
            signal_separation: None,
        };
        let mut engine = AdaptationEngine::new(config);
        let result = engine.select(&entities());
        assert!(result.is_some());
    }

    #[test]
    fn ucb1_method() {
        let config = Adaptation {
            method: AdaptationMethod::Ucb1,
            parameters: None,
            cold_start: None,
            prior_management: None,
            signal_separation: None,
        };
        let mut engine = AdaptationEngine::new(config);
        let result = engine.select(&entities());
        assert!(result.is_some());
    }

    #[test]
    fn static_method_picks_first() {
        let config = Adaptation {
            method: AdaptationMethod::Static,
            parameters: None,
            cold_start: None,
            prior_management: None,
            signal_separation: None,
        };
        let mut engine = AdaptationEngine::new(config);
        let result = engine.select(&entities());
        assert_eq!(result, Some("model_a".into()));
    }

    #[test]
    fn reset_prior_manually() {
        let mut engine = AdaptationEngine::new(thompson_config());
        for _ in 0..10 {
            engine.update(
                "model_a",
                Outcome {
                    success: true,
                    quality_score: 0.9,
                },
            );
        }
        assert!(engine.get_prior("model_a").alpha > 5.0);

        engine.reset_prior("model_a");
        assert_eq!(engine.get_prior("model_a").alpha, 2.0);
        assert_eq!(engine.observations("model_a"), 0);
    }
}
