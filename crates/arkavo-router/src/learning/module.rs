//! LearningModule - Thompson Sampling for agent/model selection

use super::agent_utility::{AgentUtility, BetaPrior};
use super::config::LearningConfig;
use super::feedback_types::{BurstFeedback, FinalTaskReport};
use super::tool_patterns::ToolCallFormat;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Statistics for an agent's utility (for external consumption)
#[derive(Debug, Clone)]
pub struct AgentUtilityStats {
    pub agent_id: String,
    pub alpha: f64,
    pub beta_param: f64,
    pub expected_value: f64,
    pub std_dev: f64,
    pub total_observations: u64,
    pub success_rate: f64,
    pub probationary: bool,
    pub total_cost_usd: f64,
    pub avg_latency_ms: f64,
}

/// Per-call cost floor (USD) used when discounting the quality sample by cost.
///
/// Local models report a $0 authored cost; dividing by that would make them
/// infinitely preferable. The floor is sized against the cheapest paid per-call
/// estimate in the authored table: DeepSeek V3.2 on the smallest task category
/// (CodeSearch: 200 input / 500 output tokens) ≈
/// (200/1M)·$0.27 + (500/1M)·$1.10 ≈ $0.0006. Rounding up to $0.001 gives
/// local/free models a bounded cost advantage (not infinite) and caps the
/// discount spread between a local and a paid arm. At the default
/// `cost_sensitivity = 0.25` the floor yields a bounded (not runaway) edge for
/// free arms. If the pricing table shifts so the cheapest paid tier changes
/// materially, `COST_FLOOR` should be revisited —
/// `test_rank_agents_cost_aware_local_not_infinitely_favored` is the tripwire.
const COST_FLOOR: f64 = 0.001;

/// Learning module for Thompson Sampling based agent selection
pub struct LearningModule {
    config: LearningConfig,
    agents: Arc<RwLock<HashMap<String, AgentUtility>>>,
    total_observations: Arc<RwLock<u64>>,
}

impl LearningModule {
    /// Create a new learning module with default config
    pub fn new() -> Self {
        Self::with_config(LearningConfig::default())
    }

    /// Access the agent utility map for analysis (e.g., curiosity scanning).
    pub async fn agents(&self) -> tokio::sync::RwLockReadGuard<'_, HashMap<String, AgentUtility>> {
        self.agents.read().await
    }

    /// Create a new learning module with custom config
    pub fn with_config(config: LearningConfig) -> Self {
        Self {
            config,
            agents: Arc::new(RwLock::new(HashMap::new())),
            total_observations: Arc::new(RwLock::new(0)),
        }
    }

    /// Get or create utility tracking for an agent
    async fn get_or_create_agent(&self, agent_id: &str) -> AgentUtility {
        let agents = self.agents.read().await;
        if let Some(utility) = agents.get(agent_id) {
            return utility.clone();
        }
        drop(agents);

        let mut agents = self.agents.write().await;
        agents
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentUtility::new(agent_id.to_string()))
            .clone()
    }

    /// Sample from Thompson Sampling for agent selection
    ///
    /// Returns a score based on sampling from the agent's Beta distribution.
    /// Higher scores indicate higher expected utility with exploration bonus.
    pub async fn sample_score(
        &self,
        agent_id: &str,
        category: Option<&str>,
        semantic_sim: f64,
        skill_overlap: f64,
    ) -> f64 {
        let utility = self.get_or_create_agent(agent_id).await;

        // Sample from Beta distribution
        let mut utility_sample = utility.sample(category);

        // Add exploration bonus for high-uncertainty agents
        let prior = utility.get_prior(category);
        if prior.std_dev() > 0.2 {
            utility_sample = self
                .config
                .exploration_bonus
                .mul_add(prior.std_dev(), utility_sample);
        }

        self.config
            .score(semantic_sim, skill_overlap, utility_sample)
    }

    /// Get raw Thompson sample for an agent (for ranking)
    ///
    /// Uses blended sampling to adapt to concept drift: blends global prior
    /// with windowed prior for faster response to performance changes.
    pub async fn thompson_sample(&self, agent_id: &str, category: Option<&str>) -> f64 {
        let utility = self.get_or_create_agent(agent_id).await;
        utility.sample_blended(category)
    }

    /// Immediate update: Record burst-level feedback
    ///
    /// Called immediately after a burst completes.
    pub async fn immediate_update(&self, agent_id: &str, feedback: &BurstFeedback) {
        let mut agents = self.agents.write().await;
        let utility = agents
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentUtility::new(agent_id.to_string()));

        utility.record_outcome(feedback, self.config.max_recent_bursts);
        utility.record_usage(feedback.cost_usd, feedback.tokens_used);
        utility.check_graduation(self.config.probation_threshold);

        // Track total observations for decay
        drop(agents);
        let mut total = self.total_observations.write().await;
        *total += 1;

        // Apply decay periodically
        if *total % self.config.decay_interval == 0 {
            drop(total);
            self.apply_decay().await;
        }
    }

    /// Retrospective update: Apply discounted credit assignment
    ///
    /// Called after a complete task finishes. Distributes reward backward
    /// through the contribution chain with discount factor γ.
    /// When `per_step_rewards` is non-empty, uses those instead of uniform
    /// discounted final_reward (backward compat when empty).
    pub async fn retrospective_update(&self, report: &FinalTaskReport) {
        let contributions = &report.agent_contributions;
        if contributions.is_empty() {
            return;
        }

        let use_per_step = report.per_step_rewards.len() == contributions.len();

        // Find max position to calculate discounts correctly (handles unsorted contributions)
        let max_position = contributions.iter().map(|c| c.position).max().unwrap_or(0);
        let mut agents = self.agents.write().await;

        // Apply discounted rewards based on actual position (later agents get more credit)
        for (idx, contribution) in contributions.iter().enumerate() {
            let utility = agents
                .entry(contribution.agent_id.clone())
                .or_insert_with(|| AgentUtility::new(contribution.agent_id.clone()));

            let combined = if use_per_step {
                // Use per-step reward directly (already incorporates quality)
                let step_reward = report.per_step_rewards[idx];
                0.5f64.mul_add(contribution.immediate_reward, 0.5 * step_reward)
            } else {
                // Discount factor raised to position from end (using actual position)
                // Last agent (position = max_position) gets γ^0 = 1.0
                // First agent (position = 0) gets γ^max_position
                let position_from_end = max_position - contribution.position;
                #[allow(clippy::cast_possible_wrap)]
                let discount = self.config.discount_factor.powi(position_from_end as i32);

                // Combine immediate reward with discounted final reward
                let immediate = contribution.immediate_reward;
                let retrospective = report.final_reward * discount;

                // Weight: 50% immediate, 50% retrospective
                0.5f64.mul_add(immediate, 0.5 * retrospective)
            };

            // Apply as fractional update (centered around 0.5)
            let weight = (combined - 0.5) * 2.0; // -1.0 to 1.0
            utility.prior.apply_fractional_update(weight);
        }
    }

    /// Apply decay to all agents for concept drift adaptation
    async fn apply_decay(&self) {
        let mut agents = self.agents.write().await;
        for utility in agents.values_mut() {
            utility.prior.decay(self.config.decay_factor);
            utility.decay_window(self.config.decay_factor);
        }
        tracing::debug!("Applied decay to {} agents", agents.len());
    }

    /// Get statistics for an agent
    pub async fn get_stats(&self, agent_id: &str) -> Option<AgentUtilityStats> {
        let agents = self.agents.read().await;
        agents.get(agent_id).map(|u| AgentUtilityStats {
            agent_id: u.agent_id.clone(),
            alpha: u.prior.alpha,
            beta_param: u.prior.beta,
            expected_value: u.prior.expected_value(),
            std_dev: u.prior.std_dev(),
            total_observations: u.total_observations(),
            success_rate: if u.total_observations() > 0 {
                u.total_successes as f64 / u.total_observations() as f64
            } else {
                0.0
            },
            probationary: u.probationary,
            total_cost_usd: u.total_cost_usd,
            avg_latency_ms: u.avg_latency_ms,
        })
    }

    /// Get statistics for all agents
    pub async fn get_all_stats(&self) -> Vec<AgentUtilityStats> {
        let agents = self.agents.read().await;
        agents
            .values()
            .map(|u| AgentUtilityStats {
                agent_id: u.agent_id.clone(),
                alpha: u.prior.alpha,
                beta_param: u.prior.beta,
                expected_value: u.prior.expected_value(),
                std_dev: u.prior.std_dev(),
                total_observations: u.total_observations(),
                success_rate: if u.total_observations() > 0 {
                    u.total_successes as f64 / u.total_observations() as f64
                } else {
                    0.0
                },
                probationary: u.probationary,
                total_cost_usd: u.total_cost_usd,
                avg_latency_ms: u.avg_latency_ms,
            })
            .collect()
    }

    /// Check if an agent is still probationary
    pub async fn is_probationary(&self, agent_id: &str) -> bool {
        let agents = self.agents.read().await;
        agents.get(agent_id).map(|u| u.probationary).unwrap_or(true)
    }

    /// Get the expected value for an agent (for deterministic ranking)
    pub async fn expected_value(&self, agent_id: &str, category: Option<&str>) -> f64 {
        let agents = self.agents.read().await;
        agents
            .get(agent_id)
            .map(|u| u.get_prior(category).expected_value())
            .unwrap_or(0.667) // Cold start expected value
    }

    /// Rank agents by Thompson Sampling for a given category
    pub async fn rank_agents(
        &self,
        agent_ids: &[String],
        category: Option<&str>,
    ) -> Vec<(String, f64)> {
        let mut scored: Vec<(String, f64)> = Vec::with_capacity(agent_ids.len());

        for agent_id in agent_ids {
            let sample = self.thompson_sample(agent_id, category).await;
            scored.push((agent_id.clone(), sample));
        }

        // Sort by score descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored
    }

    /// Rank agents by Thompson Sampling, discounting the quality sample by cost.
    ///
    /// For each feasible agent:
    /// ```text
    /// cost     = max(cost_by_agent[id], COST_FLOOR)
    /// min_cost = min over the candidate set of `cost`
    /// score    = quality_sample * (min_cost / cost) ^ cost_sensitivity
    /// ```
    ///
    /// Normalizing against the *cheapest* candidate makes the multiplier ≤ 1.0
    /// everywhere: the cheapest model is unpenalized, every other is discounted.
    /// This keeps the score in `[0, 1]` (matching the raw Thompson sample's range)
    /// and avoids the cost → 0 singularity a `quality / cost` objective would
    /// introduce. At `cost_sensitivity == 0` the cost term vanishes and the
    /// ranking is identical to [`rank_agents`](Self::rank_agents).
    ///
    /// `cost_by_agent` carries authored per-call USD estimates (see
    /// `RoutingDecision::estimate_cost`); any agent missing from the map is
    /// treated as free (floored to `COST_FLOOR`).
    pub async fn rank_agents_cost_aware(
        &self,
        agent_ids: &[String],
        category: Option<&str>,
        cost_by_agent: &HashMap<String, f64>,
    ) -> Vec<(String, f64)> {
        // Clamp at the point of use so the documented [0,1] policy holds no
        // matter how the value was set (Default, with_config, pub-field
        // mutation, or deserialized config). The field is pub and Deserialize,
        // so construction-time clamping alone cannot enforce the invariant.
        let sensitivity = self.config.cost_sensitivity.clamp(0.0, 1.0);
        if sensitivity <= 0.0 || agent_ids.is_empty() {
            // No cost discount to apply; delegate to the pure-quality path.
            return self.rank_agents(agent_ids, category).await;
        }

        // Floor each cost and find the cheapest survivor for normalization.
        let floored: Vec<(String, f64)> = agent_ids
            .iter()
            .map(|id| {
                let c = cost_by_agent
                    .get(id)
                    .copied()
                    .unwrap_or(0.0)
                    .max(COST_FLOOR);
                (id.clone(), c)
            })
            .collect();
        let min_cost = floored
            .iter()
            .map(|(_, c)| *c)
            .fold(f64::INFINITY, f64::min)
            .max(COST_FLOOR);

        let mut scored: Vec<(String, f64)> = Vec::with_capacity(agent_ids.len());
        for (agent_id, cost) in &floored {
            let quality = self.thompson_sample(agent_id, category).await;
            // min_cost / cost ∈ (0, 1]; raised to `sensitivity` keeps it ≤ 1.
            let discount = (min_cost / cost).powf(sensitivity);
            scored.push((agent_id.clone(), quality * discount));
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored
    }

    /// Select an agent with guaranteed probationary budget allocation
    ///
    /// Ensures probationary agents receive at least `probationary_budget_ratio`
    /// portion of selections, enabling exploration of new/unknown agents.
    ///
    /// Returns `(agent_id, score, was_probation_selected)` where `was_probation_selected`
    /// is true if a probationary agent was force-selected for exploration.
    pub async fn select_with_probation_guarantee(
        &self,
        agent_ids: &[String],
        category: Option<&str>,
    ) -> Option<(String, f64, bool)> {
        if agent_ids.is_empty() {
            return None;
        }

        let agents = self.agents.read().await;

        // Collect probationary agents
        let probationary: Vec<_> = agent_ids
            .iter()
            .filter(|id| agents.get(*id).map(|u| u.probationary).unwrap_or(true))
            .collect();

        drop(agents);

        // With probability probationary_budget_ratio, select a probationary agent
        if !probationary.is_empty() {
            use rand::Rng;
            // Generate random values before any await points to avoid Send issues
            let (should_select_probationary, idx) = {
                let mut rng = rand::thread_rng();
                let should = rng.r#gen::<f64>() < self.config.probationary_budget_ratio;
                let idx = rng.gen_range(0..probationary.len());
                (should, idx)
            };

            if should_select_probationary {
                // Force selection of a random probationary agent
                let agent_id = probationary[idx].clone();
                let score = self.thompson_sample(&agent_id, category).await;
                return Some((agent_id, score, true));
            }
        }

        // Normal Thompson Sampling selection
        let ranked = self.rank_agents(agent_ids, category).await;
        ranked
            .first()
            .map(|(id, score)| (id.clone(), *score, false))
    }

    /// Select the best tool call format for an agent using Thompson Sampling
    ///
    /// Uses the existing category_priors mechanism with "format:<type>" keys
    /// to learn which formats work best for each agent/model.
    ///
    /// Returns `(format, score)` where score is the Thompson sample.
    pub async fn sample_format(&self, agent_id: &str) -> (ToolCallFormat, f64) {
        let mut best_format = ToolCallFormat::Fence; // Default
        let mut best_score = 0.0;

        for format in ToolCallFormat::all() {
            let category = format.to_category_key();
            let score = self.thompson_sample(agent_id, Some(&category)).await;

            if score > best_score {
                best_score = score;
                best_format = *format;
            }
        }

        (best_format, best_score)
    }

    /// Seed category priors for an agent (warm start from static heuristic)
    ///
    /// Only inserts priors that don't already exist, preserving
    /// any persisted state from previous runs.
    pub async fn seed_priors(&self, agent_id: &str, priors: &[(&str, f64, f64)]) {
        let mut agents = self.agents.write().await;
        let utility = agents
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentUtility::new(agent_id.to_string()));

        for &(category, alpha, beta) in priors {
            utility
                .category_priors
                .entry(category.to_string())
                .or_insert_with(|| BetaPrior::new(alpha, beta));
        }
    }

    /// Get per-category statistics for an agent
    ///
    /// Returns `(category, alpha, beta, expected_value, observations)` tuples,
    /// filtering out `"format:"` keys which are used for tool format learning.
    pub async fn get_category_stats(&self, agent_id: &str) -> Vec<(String, f64, f64, f64, u64)> {
        let agents = self.agents.read().await;
        let Some(utility) = agents.get(agent_id) else {
            return vec![];
        };

        utility
            .category_priors
            .iter()
            .filter(|(k, _)| !k.starts_with("format:"))
            .map(|(k, prior)| {
                (
                    k.clone(),
                    prior.alpha,
                    prior.beta,
                    prior.expected_value(),
                    prior.total_observations() as u64,
                )
            })
            .collect()
    }

    /// Get format statistics for an agent
    ///
    /// Returns a map of format -> (expected_value, observations) for analysis.
    pub async fn get_format_stats(&self, agent_id: &str) -> HashMap<ToolCallFormat, (f64, u64)> {
        let agents = self.agents.read().await;
        let mut stats = HashMap::new();

        if let Some(utility) = agents.get(agent_id) {
            for format in ToolCallFormat::all() {
                let key = format.to_category_key();
                if let Some(prior) = utility.category_priors.get(&key) {
                    let obs = prior.total_observations() as u64;
                    stats.insert(*format, (prior.expected_value(), obs));
                }
            }
        }

        stats
    }
}

impl Default for LearningModule {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::AgentContribution;
    use arkavo_test_macros::spec;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_cold_start_sampling() {
        let module = LearningModule::new();

        // New agent should sample around 0.667 (Beta(2,1) mean)
        let mut samples = Vec::new();
        for _ in 0..100 {
            samples.push(module.thompson_sample("new-agent", None).await);
        }

        let mean: f64 = samples.iter().sum::<f64>() / samples.len() as f64;
        assert!(mean > 0.5 && mean < 0.8, "Mean should be around 0.667");
    }

    #[tokio::test]
    async fn test_immediate_update_success() {
        let module = LearningModule::new();

        let feedback = BurstFeedback::success(Uuid::new_v4(), "test".to_string(), 100);

        module.immediate_update("agent-1", &feedback).await;

        let stats = module.get_stats("agent-1").await.unwrap();
        assert_eq!(stats.total_observations, 1);
        assert!((stats.success_rate - 1.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_immediate_update_failure() {
        let module = LearningModule::new();

        let feedback = BurstFeedback::failure(Uuid::new_v4(), "test".to_string(), 100);

        module.immediate_update("agent-1", &feedback).await;

        let stats = module.get_stats("agent-1").await.unwrap();
        assert_eq!(stats.total_observations, 1);
        assert!((stats.success_rate - 0.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_retrospective_update() {
        let module = LearningModule::new();

        // First, create agents with some baseline
        for agent in ["agent-1", "agent-2", "agent-3"] {
            let feedback = BurstFeedback::success(Uuid::new_v4(), "test".to_string(), 100);
            module.immediate_update(agent, &feedback).await;
        }

        // Now apply retrospective update
        let report = FinalTaskReport::success(
            Uuid::new_v4(),
            vec![
                AgentContribution {
                    agent_id: "agent-1".to_string(),
                    position: 0,
                    immediate_reward: 0.5,
                },
                AgentContribution {
                    agent_id: "agent-2".to_string(),
                    position: 1,
                    immediate_reward: 0.8,
                },
                AgentContribution {
                    agent_id: "agent-3".to_string(),
                    position: 2,
                    immediate_reward: 1.0,
                },
            ],
        );

        module.retrospective_update(&report).await;

        // Agent-3 (last) should have highest expected value
        let ev1 = module.expected_value("agent-1", None).await;
        let ev3 = module.expected_value("agent-3", None).await;
        assert!(ev3 > ev1, "Last agent should have higher expected value");
    }

    #[tokio::test]
    async fn test_retrospective_update_unsorted_contributions() {
        let module = LearningModule::new();

        // Create agents with baseline
        for agent in ["agent-first", "agent-mid", "agent-last"] {
            let feedback = BurstFeedback::success(Uuid::new_v4(), "test".to_string(), 100);
            module.immediate_update(agent, &feedback).await;
        }

        // Contributions in WRONG vector order (position 2, 0, 1)
        // This tests that we use contribution.position, not vector index
        let report = FinalTaskReport::success(
            Uuid::new_v4(),
            vec![
                AgentContribution {
                    agent_id: "agent-last".to_string(),
                    position: 2, // Last in sequence
                    immediate_reward: 0.5,
                },
                AgentContribution {
                    agent_id: "agent-first".to_string(),
                    position: 0, // First in sequence
                    immediate_reward: 0.5,
                },
                AgentContribution {
                    agent_id: "agent-mid".to_string(),
                    position: 1, // Middle
                    immediate_reward: 0.5,
                },
            ],
        );

        module.retrospective_update(&report).await;

        // agent-last (position=2) should have highest EV despite being first in vector
        let ev_first = module.expected_value("agent-first", None).await;
        let ev_mid = module.expected_value("agent-mid", None).await;
        let ev_last = module.expected_value("agent-last", None).await;

        assert!(
            ev_last > ev_mid,
            "Last position agent should have higher EV than middle"
        );
        assert!(
            ev_mid > ev_first,
            "Middle position agent should have higher EV than first"
        );
    }

    #[tokio::test]
    async fn test_probation_graduation() {
        let config = LearningConfig::default().with_probation_threshold(5);
        let module = LearningModule::with_config(config);

        // Check initially probationary
        assert!(module.is_probationary("agent-1").await);

        // Add 5 observations
        for _ in 0..5 {
            let feedback = BurstFeedback::success(Uuid::new_v4(), "test".to_string(), 100);
            module.immediate_update("agent-1", &feedback).await;
        }

        // Should graduate
        assert!(!module.is_probationary("agent-1").await);
    }

    #[tokio::test]
    async fn test_rank_agents() {
        let module = LearningModule::new();

        // Create agents with different success rates
        for _ in 0..10 {
            module
                .immediate_update(
                    "good-agent",
                    &BurstFeedback::success(Uuid::new_v4(), "test".to_string(), 100),
                )
                .await;
        }

        for _ in 0..10 {
            module
                .immediate_update(
                    "bad-agent",
                    &BurstFeedback::failure(Uuid::new_v4(), "test".to_string(), 100),
                )
                .await;
        }

        // Rank them multiple times - good agent should usually be first
        let mut good_first = 0;
        for _ in 0..20 {
            let ranked = module
                .rank_agents(&["good-agent".to_string(), "bad-agent".to_string()], None)
                .await;
            if ranked[0].0 == "good-agent" {
                good_first += 1;
            }
        }

        assert!(
            good_first > 15,
            "Good agent should be ranked first most of the time"
        );
    }

    #[spec("ROUTER-018")]
    #[tokio::test]
    async fn test_rank_agents_cost_aware_favors_cheaper_equal_quality() {
        let module = LearningModule::new();

        // Two agents with identical, well-established priors (50 successes each).
        for agent in ["cheap", "pricey"] {
            for _ in 0..50 {
                module
                    .immediate_update(
                        agent,
                        &BurstFeedback::success(Uuid::new_v4(), "test".to_string(), 100),
                    )
                    .await;
            }
        }

        let costs = HashMap::from([("cheap".to_string(), 0.001), ("pricey".to_string(), 0.10)]);

        let mut cheap_first = 0;
        for _ in 0..20 {
            let ranked = module
                .rank_agents_cost_aware(&["cheap".to_string(), "pricey".to_string()], None, &costs)
                .await;
            if ranked[0].0 == "cheap" {
                cheap_first += 1;
            }
        }

        assert!(
            cheap_first > 15,
            "At equal quality the cheaper agent should win most of the time; won {cheap_first}/20"
        );
    }

    #[spec("ROUTER-018")]
    #[tokio::test]
    async fn test_rank_agents_cost_aware_quality_dominates_at_zero_sensitivity() {
        let config = LearningConfig {
            cost_sensitivity: 0.0,
            ..Default::default()
        };
        let module = LearningModule::with_config(config);

        // well-established priors so sampling is stable.
        for _ in 0..50 {
            module
                .immediate_update(
                    "good",
                    &BurstFeedback::success(Uuid::new_v4(), "test".to_string(), 100),
                )
                .await;
            module
                .immediate_update(
                    "bad",
                    &BurstFeedback::failure(Uuid::new_v4(), "test".to_string(), 100),
                )
                .await;
        }

        // `good` is far cheaper too — but at zero sensitivity cost is irrelevant,
        // so ranking must match the pure-quality `rank_agents` order.
        let costs = HashMap::from([("good".to_string(), 0.001), ("bad".to_string(), 0.10)]);

        let ids = ["good".to_string(), "bad".to_string()];
        let mut agree = 0;
        for _ in 0..20 {
            let plain = module.rank_agents(&ids, None).await;
            let costed = module.rank_agents_cost_aware(&ids, None, &costs).await;
            if plain[0].0 == costed[0].0 {
                agree += 1;
            }
        }
        assert!(
            agree == 20,
            "At cost_sensitivity=0 the cost-aware ranking must equal rank_agents on every draw"
        );
    }

    #[spec("ROUTER-018")]
    #[tokio::test]
    async fn test_rank_agents_cost_aware_clamps_out_of_range_sensitivity() {
        // Regression: cost_sensitivity is clamped to [0,1] at the point of use,
        // so a pub-field mutation or deserialized config value outside the range
        // is coerced, not silently honored. A value > 1.0 must behave as 1.0,
        // and a negative value must behave as 0.0 (pure quality).
        //
        // Setup makes the two arms competitive *at s=1.0* so a clamp to 1.0 is
        // observable: pricey has ~2x the quality of cheap, and a 2x cost spread.
        // At s=1.0 the discount balances the quality gap (~50/50 split); at an
        // unclamped s=5.0 the discount would crush pricey to ~0 wins.
        async fn build(cost_sensitivity: f64, cheap_quality: f64) -> LearningModule {
            let module = LearningModule::with_config(LearningConfig {
                cost_sensitivity,
                ..Default::default()
            });
            // Pricey: strong prior (~0.98 mean). Cheap: weaker prior matching
            // `cheap_quality` via fractional successes/failures.
            for _ in 0..50 {
                module
                    .immediate_update(
                        "pricey",
                        &BurstFeedback::success(Uuid::new_v4(), "test".to_string(), 100),
                    )
                    .await;
            }
            let cheap_successes = (50.0 * cheap_quality).round() as usize;
            let cheap_failures = 50 - cheap_successes;
            for _ in 0..cheap_successes {
                module
                    .immediate_update(
                        "cheap",
                        &BurstFeedback::success(Uuid::new_v4(), "test".to_string(), 100),
                    )
                    .await;
            }
            for _ in 0..cheap_failures {
                module
                    .immediate_update(
                        "cheap",
                        &BurstFeedback::failure(Uuid::new_v4(), "test".to_string(), 100),
                    )
                    .await;
            }
            module
        }

        // 2x cost spread; cheap quality ~0.49 so its mean (~0.49) × 1.0 ≈ pricey
        // mean (~0.98) × (1/2)^1 = 0.49 → near-parity at s=1.0.
        let costs = HashMap::from([("cheap".to_string(), 0.001), ("pricey".to_string(), 0.002)]);
        let ids = ["cheap".to_string(), "pricey".to_string()];

        // > 1.0 must clamp to 1.0: pricey stays competitive (wins a meaningful
        // share). If the value were honored as 5.0, pricey would win ~0.
        let over = build(5.0, 0.49).await;
        let mut pricey_wins_over = 0;
        for _ in 0..60 {
            let ranked = over.rank_agents_cost_aware(&ids, None, &costs).await;
            if ranked[0].0 == "pricey" {
                pricey_wins_over += 1;
            }
        }
        assert!(
            pricey_wins_over >= 10,
            "cost_sensitivity=5.0 should clamp to 1.0, keeping pricey competitive; \
             got {pricey_wins_over}/60"
        );

        // Negative must clamp to 0.0 (pure quality): pricey's far higher quality
        // then dominates, so pricey should win the large majority. If the cost
        // term were active at any nonzero value, cheap would steal wins.
        let neg = build(-3.0, 0.49).await;
        let mut pricey_wins_neg = 0;
        for _ in 0..60 {
            let ranked = neg.rank_agents_cost_aware(&ids, None, &costs).await;
            if ranked[0].0 == "pricey" {
                pricey_wins_neg += 1;
            }
        }
        assert!(
            pricey_wins_neg >= 50,
            "negative cost_sensitivity should clamp to 0.0, letting the higher-quality \
             pricey arm dominate; got {pricey_wins_neg}/60"
        );
    }

    #[spec("ROUTER-018")]
    #[tokio::test]
    async fn test_rank_agents_cost_aware_cold_start_band() {
        // Regression for the reviewer's hazard: a cheap, under-observed arm must
        // neither monopolize selection nor be starved. We assert a BAND, not a
        // one-sided threshold, so the test fails both ways.
        //
        // Setup is tuned so the two arms are near-parity in *expected* score,
        // making the outcome genuinely sample-dependent (a real coin-flip band):
        //   established: Beta(42,11) → mean ≈ 0.79, cost = 0.002 (expensive)
        //   cheap-cold : Beta(2,1)   → mean ≈ 0.667, cost = 0.001 (cheap)
        // At cost_sensitivity = 0.25 the established arm's expected discounted
        // score is 0.79·(0.001/0.002)^0.25 ≈ 0.667 — matching the cold arm's
        // mean. So selection swings with the Thompson draw, not the cost.
        let module = LearningModule::new();

        for _ in 0..40 {
            module
                .immediate_update(
                    "established",
                    &BurstFeedback::success(Uuid::new_v4(), "test".to_string(), 100),
                )
                .await;
        }
        for _ in 0..10 {
            module
                .immediate_update(
                    "established",
                    &BurstFeedback::failure(Uuid::new_v4(), "test".to_string(), 100),
                )
                .await;
        }

        // Cheap arm stays at the cold-start prior (no observations) → wide Beta.
        let costs = HashMap::from([
            ("established".to_string(), 0.002),
            ("cheap-cold".to_string(), 0.001),
        ]);
        let ids = ["established".to_string(), "cheap-cold".to_string()];

        let mut cheap_selected = 0;
        const N: u32 = 60;
        for _ in 0..N {
            let ranked = module.rank_agents_cost_aware(&ids, None, &costs).await;
            if ranked[0].0 == "cheap-cold" {
                cheap_selected += 1;
            }
        }

        let rate = f64::from(cheap_selected) / f64::from(N);
        // Two-sided band: the cold arm is explored (rate ≥ 0.20) but does not
        // crowd out the established arm (rate ≤ 0.80). A one-sided threshold
        // could pass trivially on a lucky seed without proving exploration.
        assert!(
            (0.20..=0.80).contains(&rate),
            "cheap cold-start arm selected {cheap_selected}/{N} (rate {rate:.2}); \
             expected between 0.20 and 0.80"
        );
    }

    #[spec("ROUTER-018")]
    #[tokio::test]
    async fn test_rank_agents_cost_aware_local_not_infinitely_favored() {
        // Pins COST_FLOOR: a low-quality local (free) model must NOT beat a
        // much-higher-quality paid model merely because it's cheap. If the
        // pricing table shifts so the cheapest paid tier changes materially,
        // this test is the tripwire.
        let module = LearningModule::new();

        // High-quality paid arm.
        for _ in 0..50 {
            module
                .immediate_update(
                    "paid-strong",
                    &BurstFeedback::success(Uuid::new_v4(), "test".to_string(), 100),
                )
                .await;
        }
        // Low-quality local arm.
        for _ in 0..50 {
            module
                .immediate_update(
                    "local-weak",
                    &BurstFeedback::failure(Uuid::new_v4(), "test".to_string(), 100),
                )
                .await;
        }

        // Local is free (floored to COST_FLOOR); paid is ~100x the floor.
        let costs = HashMap::from([
            ("paid-strong".to_string(), 0.10),
            ("local-weak".to_string(), 0.0),
        ]);
        let ids = ["paid-strong".to_string(), "local-weak".to_string()];

        let mut paid_first = 0;
        for _ in 0..20 {
            let ranked = module.rank_agents_cost_aware(&ids, None, &costs).await;
            if ranked[0].0 == "paid-strong" {
                paid_first += 1;
            }
        }

        assert!(
            paid_first >= 15,
            "A much-higher-quality paid model should still outrank a weak free one; \
             paid won {paid_first}/20"
        );
    }

    #[tokio::test]
    async fn test_probation_guarantee_selects_new_agents() {
        let config = LearningConfig {
            probationary_budget_ratio: 1.0, // Force probation selection
            ..Default::default()
        };
        let module = LearningModule::with_config(config);

        // Create one established agent (50+ observations)
        for _ in 0..50 {
            module
                .immediate_update(
                    "established",
                    &BurstFeedback::success(Uuid::new_v4(), "test".to_string(), 100),
                )
                .await;
        }

        // Select from pool including a new agent
        let result = module
            .select_with_probation_guarantee(
                &["established".to_string(), "new-agent".to_string()],
                None,
            )
            .await;

        let (selected, _, was_probation) = result.unwrap();
        assert_eq!(selected, "new-agent");
        assert!(was_probation);
    }

    #[tokio::test]
    async fn test_probation_guarantee_zero_ratio_uses_thompson() {
        let config = LearningConfig {
            probationary_budget_ratio: 0.0, // Never force probation
            ..Default::default()
        };
        let module = LearningModule::with_config(config);

        // Create a high-performing agent
        for _ in 0..20 {
            module
                .immediate_update(
                    "good-agent",
                    &BurstFeedback::success(Uuid::new_v4(), "test".to_string(), 100),
                )
                .await;
        }

        // Select should usually pick good-agent via Thompson Sampling
        let mut good_selected = 0;
        for _ in 0..10 {
            if let Some((id, _, was_probation)) = module
                .select_with_probation_guarantee(
                    &["good-agent".to_string(), "unknown".to_string()],
                    None,
                )
                .await
            {
                if id == "good-agent" {
                    good_selected += 1;
                }
                assert!(!was_probation); // Never forced
            }
        }
        assert!(
            good_selected >= 5,
            "Good agent should be selected most times"
        );
    }

    #[tokio::test]
    async fn test_format_learning_via_categories() {
        use crate::learning::ToolCallFormat;

        let module = LearningModule::new();

        // Record successes for fence format
        for _ in 0..10 {
            module
                .immediate_update(
                    "model-a",
                    &BurstFeedback::success(
                        Uuid::new_v4(),
                        ToolCallFormat::Fence.to_category_key(),
                        100,
                    ),
                )
                .await;
        }

        // Record failures for xml format
        for _ in 0..10 {
            module
                .immediate_update(
                    "model-a",
                    &BurstFeedback::failure(
                        Uuid::new_v4(),
                        ToolCallFormat::Xml.to_category_key(),
                        100,
                    ),
                )
                .await;
        }

        // Sample format - fence should win most of the time
        let mut fence_wins = 0;
        for _ in 0..20 {
            let (format, _) = module.sample_format("model-a").await;
            if format == ToolCallFormat::Fence {
                fence_wins += 1;
            }
        }

        assert!(fence_wins > 10, "Fence should be selected most times");
    }

    #[tokio::test]
    async fn test_get_format_stats() {
        use crate::learning::ToolCallFormat;

        let module = LearningModule::new();

        // Add some format observations
        for _ in 0..5 {
            module
                .immediate_update(
                    "model-b",
                    &BurstFeedback::success(
                        Uuid::new_v4(),
                        ToolCallFormat::Json.to_category_key(),
                        100,
                    ),
                )
                .await;
        }

        let stats = module.get_format_stats("model-b").await;
        assert!(stats.contains_key(&ToolCallFormat::Json));

        let (ev, obs) = stats[&ToolCallFormat::Json];
        assert!(
            ev > 0.6,
            "Expected value should be high for successful format"
        );
        assert_eq!(obs, 5);
    }
}
