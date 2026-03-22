//! Curiosity-driven task generation (Synaptic Phase 5).
//!
//! Scans the agent × category Beta prior matrix for high-variance cells
//! (uncertainty gaps) and generates tasks targeting those gaps. This is
//! intrinsic motivation — the mesh invents its own curriculum by
//! maximizing information gain on the capability matrix.

use arkavo_router::learning::AgentUtility;
use std::collections::HashMap;

/// Minimum variance to consider a cell as an uncertainty gap.
const MIN_VARIANCE_THRESHOLD: f64 = 0.02;

/// Maximum tasks to generate per curiosity scan.
const MAX_CURIOSITY_TASKS: usize = 3;

/// An uncertainty gap in the agent × category matrix.
#[derive(Debug, Clone)]
pub(super) struct UncertaintyGap {
    pub agent_id: String,
    pub category: String,
    pub variance: f64,
    pub observations: f64,
    pub expected_value: f64,
}

/// Scan agent utilities for high-variance cells (uncertainty gaps).
///
/// Returns the top gaps sorted by variance descending — these are
/// the areas where additional tasks would yield the most information gain.
pub(super) fn find_uncertainty_gaps(agents: &HashMap<String, AgentUtility>) -> Vec<UncertaintyGap> {
    let mut gaps = Vec::new();

    for (agent_id, utility) in agents {
        // Check the global prior
        let global = &utility.prior;
        if global.variance() >= MIN_VARIANCE_THRESHOLD && global.total_observations() < 10.0 {
            gaps.push(UncertaintyGap {
                agent_id: agent_id.clone(),
                category: "general".to_string(),
                variance: global.variance(),
                observations: global.total_observations(),
                expected_value: global.expected_value(),
            });
        }

        // Check each category prior
        for (category, prior) in &utility.category_priors {
            if prior.variance() >= MIN_VARIANCE_THRESHOLD {
                gaps.push(UncertaintyGap {
                    agent_id: agent_id.clone(),
                    category: category.clone(),
                    variance: prior.variance(),
                    observations: prior.total_observations(),
                    expected_value: prior.expected_value(),
                });
            }
        }
    }

    // Sort by variance descending — highest uncertainty first
    gaps.sort_by(|a, b| {
        b.variance
            .partial_cmp(&a.variance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    gaps.truncate(MAX_CURIOSITY_TASKS);
    gaps
}

/// Generate a task prompt targeting an uncertainty gap.
pub(super) fn generate_curiosity_task(gap: &UncertaintyGap) -> String {
    let confidence_desc = if gap.expected_value > 0.7 {
        "likely capable but unverified"
    } else if gap.expected_value > 0.4 {
        "uncertain capability"
    } else {
        "likely weak area"
    };

    let observation_desc = if gap.observations < 2.0 {
        "almost no data"
    } else if gap.observations < 5.0 {
        "very few observations"
    } else {
        "limited observations"
    };

    format!(
        "Exploration task for {agent} in category '{category}': \
         This is a {confidence} area with {observations}. \
         Generate a targeted task to test {agent}'s ability in {category} \
         and observe the outcome quality.",
        agent = gap.agent_id,
        category = gap.category,
        confidence = confidence_desc,
        observations = observation_desc,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_router::learning::BetaPrior;

    fn make_utility(agent_id: &str, categories: &[(&str, f64, f64)]) -> AgentUtility {
        let mut u = AgentUtility::new(agent_id.to_string());
        for (cat, alpha, beta) in categories {
            u.category_priors
                .insert(cat.to_string(), BetaPrior::new(*alpha, *beta));
        }
        u
    }

    #[test]
    fn finds_high_variance_gaps() {
        let mut agents = HashMap::new();
        // Low observations = high variance
        agents.insert(
            "agent1".to_string(),
            make_utility("agent1", &[("code", 2.0, 1.0)]),
        );
        // High observations = low variance
        agents.insert(
            "agent2".to_string(),
            make_utility("agent2", &[("code", 50.0, 20.0)]),
        );

        let gaps = find_uncertainty_gaps(&agents);
        assert!(
            gaps.iter().any(|g| g.agent_id == "agent1"),
            "Should find agent1 with high variance"
        );
        assert!(
            !gaps
                .iter()
                .any(|g| g.agent_id == "agent2" && g.category == "code"),
            "Should not find agent2 (low variance)"
        );
    }

    #[test]
    fn sorted_by_variance() {
        let mut agents = HashMap::new();
        agents.insert(
            "a".to_string(),
            make_utility("a", &[("x", 2.0, 1.0), ("y", 3.0, 1.0)]),
        );

        let gaps = find_uncertainty_gaps(&agents);
        if gaps.len() >= 2 {
            assert!(gaps[0].variance >= gaps[1].variance);
        }
    }

    #[test]
    fn max_tasks_limit() {
        let mut agents = HashMap::new();
        for i in 0..10 {
            agents.insert(
                format!("agent{i}"),
                make_utility(&format!("agent{i}"), &[("cat", 2.0, 1.0)]),
            );
        }

        let gaps = find_uncertainty_gaps(&agents);
        assert!(gaps.len() <= MAX_CURIOSITY_TASKS);
    }

    #[test]
    fn generates_task_prompt() {
        let gap = UncertaintyGap {
            agent_id: "historian".to_string(),
            category: "analysis".to_string(),
            variance: 0.05,
            observations: 1.0,
            expected_value: 0.6,
        };
        let prompt = generate_curiosity_task(&gap);
        assert!(prompt.contains("historian"));
        assert!(prompt.contains("analysis"));
    }

    #[test]
    fn empty_agents() {
        let agents = HashMap::new();
        let gaps = find_uncertainty_gaps(&agents);
        assert!(gaps.is_empty());
    }
}
