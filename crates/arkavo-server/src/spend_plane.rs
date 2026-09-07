//! Spend-plane wiring shared across the server and CLI entry points.
//!
//! Resolving the cloud-spend policy from an agent's AGENTS.md was duplicated in
//! every place that builds a Router (the A2A server, `LocalEngine`, and the CLI
//! tool integration). It lives here so all entry points share one source of
//! truth and the safe default is applied consistently.

use std::sync::Arc;

use arkavo_budget::{BudgetConfig, BudgetTracker, CloudPolicy};
use arkavo_router::{AgentConfig, BudgetYamlConfig};

/// Resolve the cloud-spend policy from an agent's AGENTS.md budget block,
/// falling back to the safe `AskBeforeCloud` default when it is absent or
/// unparseable.
pub fn cloud_policy_from_config(config: &AgentConfig) -> CloudPolicy {
    config
        .budget
        .as_ref()
        .and_then(|b| b.cloud_policy.as_deref())
        .and_then(CloudPolicy::parse)
        .unwrap_or_default()
}

/// Resolve the cloud-spend policy from the AGENTS.md discovered in the current
/// directory tree (or the default when none is found / it does not parse).
pub fn cloud_policy_from_agents_md() -> CloudPolicy {
    cloud_policy_from_config(&arkavo_router::load_agent_config().unwrap_or_default())
}

/// Build budget limits and posture from an AGENTS.md budget block.
///
/// Limits the block leaves out keep the crate defaults (a $10 session cap, $50
/// daily), so an agent that configures nothing still runs against a real ceiling.
pub fn budget_config_from_yaml(yaml: &BudgetYamlConfig) -> BudgetConfig {
    let mut config = BudgetConfig::default();

    if let Some(session_cost) = yaml.max_cost_per_session {
        config.limits.session_limit = Some(arkavo_budget::TokenCost::from_dollars(session_cost));
    }
    if let Some(daily_cost) = yaml.max_cost_per_day {
        config.limits.daily_limit = Some(arkavo_budget::TokenCost::from_dollars(daily_cost));
    }
    if let Some(policy) = yaml
        .cloud_policy
        .as_deref()
        .and_then(arkavo_budget::CloudPolicy::parse)
    {
        config.cloud_policy = policy;
    }

    config
}

/// Budget configuration from the AGENTS.md discovered in the current directory
/// tree, or the crate defaults when none is found.
pub fn budget_config_from_agents_md() -> BudgetConfig {
    arkavo_router::load_agent_config()
        .unwrap_or_default()
        .budget
        .as_ref()
        .map_or_else(BudgetConfig::default, budget_config_from_yaml)
}

/// A tracker carrying the configured caps, for a command that owns its own
/// spend plane instead of inheriting a long-lived one from a Router.
pub async fn budget_tracker_from_agents_md() -> anyhow::Result<Arc<BudgetTracker>> {
    BudgetTracker::new(budget_config_from_agents_md())
        .await
        .map(Arc::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_router::BudgetYamlConfig;

    fn config_with_policy(policy: Option<&str>) -> AgentConfig {
        AgentConfig {
            budget: Some(BudgetYamlConfig {
                max_cost_per_session: None,
                max_cost_per_day: None,
                cloud_policy: policy.map(str::to_string),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn parses_explicit_policy() {
        assert_eq!(
            cloud_policy_from_config(&config_with_policy(Some("local_only"))),
            CloudPolicy::LocalOnly
        );
        assert_eq!(
            cloud_policy_from_config(&config_with_policy(Some("cloud_within_cap"))),
            CloudPolicy::CloudWithinCap
        );
    }

    #[test]
    fn defaults_when_absent_or_unparseable() {
        assert_eq!(
            cloud_policy_from_config(&config_with_policy(None)),
            CloudPolicy::AskBeforeCloud
        );
        assert_eq!(
            cloud_policy_from_config(&config_with_policy(Some("nonsense"))),
            CloudPolicy::AskBeforeCloud
        );
        assert_eq!(
            cloud_policy_from_config(&AgentConfig::default()),
            CloudPolicy::AskBeforeCloud
        );
    }

    #[test]
    fn budget_yaml_sets_caps_and_keeps_defaults_for_the_rest() {
        let config = budget_config_from_yaml(&BudgetYamlConfig {
            max_cost_per_session: Some(2.5),
            max_cost_per_day: None,
            cloud_policy: Some("cloud_within_cap".into()),
        });
        assert_eq!(
            config.limits.session_limit,
            Some(arkavo_budget::TokenCost::from_dollars(2.5))
        );
        assert_eq!(
            config.limits.daily_limit,
            arkavo_budget::BudgetLimits::default().daily_limit
        );
        assert_eq!(config.cloud_policy, CloudPolicy::CloudWithinCap);
    }

    #[test]
    fn budget_yaml_without_a_policy_keeps_the_safe_default() {
        let config = budget_config_from_yaml(&BudgetYamlConfig {
            max_cost_per_session: None,
            max_cost_per_day: Some(7.0),
            cloud_policy: Some("nonsense".into()),
        });
        assert_eq!(
            config.limits.daily_limit,
            Some(arkavo_budget::TokenCost::from_dollars(7.0))
        );
        assert_eq!(config.cloud_policy, CloudPolicy::AskBeforeCloud);
    }
}
