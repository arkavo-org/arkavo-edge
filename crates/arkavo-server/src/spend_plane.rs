//! Spend-plane wiring shared across the server and CLI entry points.
//!
//! Resolving the cloud-spend policy from the discovered SwarmKit `runtime`
//! block was duplicated in every place that builds a Router (the A2A server,
//! `LocalEngine`, and the CLI tool integration). It lives here so all entry
//! points share one source of truth and the safe default is applied
//! consistently.

use arkavo_budget::CloudPolicy;
use arkavo_router::AgentConfig;

/// Resolve the cloud-spend policy from agent budget config,
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

/// Resolve the cloud-spend policy from the SwarmKit kit discovered in the
/// current directory tree (or the default when none is found).
pub fn cloud_policy_from_kit() -> CloudPolicy {
    cloud_policy_from_config(&arkavo_router::load_agent_config().unwrap_or_default())
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
}
