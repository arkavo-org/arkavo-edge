//! LearningModule - Thompson Sampling for agent/model selection

use super::agent_utility::{AgentUtility, BurstFeedback, FinalTaskReport};
use super::config::LearningConfig;
use super::tool_patterns::ToolCallFormat;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Statistics for an agent's utility (for external consumption)
#[derive(Debug, Clone)]
pub struct AgentUtilityStats {
    pub agent_id: String,
    pub expected_value: f64,
    pub std_dev: f64,
    pub total_observations: u64,
    pub success_rate: f64,
    pub probationary: bool,
    pub total_cost_usd: f64,
    pub avg_latency_ms: f64,
}

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
            utility_sample += self.config.exploration_bonus * prior.std_dev();
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
    pub async fn retrospective_update(&self, report: &FinalTaskReport) {
        let contributions = &report.agent_contributions;
        if contributions.is_empty() {
            return;
        }

        // Find max position to calculate discounts correctly (handles unsorted contributions)
        let max_position = contributions.iter().map(|c| c.position).max().unwrap_or(0);
        let mut agents = self.agents.write().await;

        // Apply discounted rewards based on actual position (later agents get more credit)
        for contribution in contributions {
            let utility = agents
                .entry(contribution.agent_id.clone())
                .or_insert_with(|| AgentUtility::new(contribution.agent_id.clone()));

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
            let combined = 0.5f64.mul_add(immediate, 0.5 * retrospective);

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
