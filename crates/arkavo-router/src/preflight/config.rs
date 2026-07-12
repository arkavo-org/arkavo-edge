//! Runtime policy configuration for preflight moderation
//!
//! Policies are loaded from the kit-level SwarmKit `runtime.preflight` block.
//! Product AGENTS.md is no longer read for process config (migrate to
//! `*.swarmkit.yaml`).

use super::features::PreflightFeature;
use super::moderator::{PolicyId, PreflightModerator};
use serde::{Deserialize, Serialize};
use torg_core::{BoolOp, Graph, Node, Source};

/// Policy action type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PolicyAction {
    /// Block requests matching the features (NOT circuit)
    #[default]
    Block,
    /// Allow only requests matching the features (pass-through)
    Allow,
}

/// Single policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// Unique policy identifier
    pub id: String,
    /// Features to evaluate
    pub features: Vec<String>,
    /// Action to take (block or allow)
    #[serde(default)]
    pub action: PolicyAction,
    /// Optional description
    pub description: Option<String>,
    /// Whether policy is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Root configuration file structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyFileConfig {
    /// List of policies
    #[serde(default)]
    pub policies: Vec<PolicyConfig>,
}

/// Parse feature string to PreflightFeature
fn parse_feature(s: &str) -> Option<PreflightFeature> {
    match s {
        "InputContainsPII" => Some(PreflightFeature::InputContainsPII),
        "InputContainsProfanity" => Some(PreflightFeature::InputContainsProfanity),
        "InputContainsSQLKeywords" => Some(PreflightFeature::InputContainsSQLKeywords),
        "InputContainsShellCommands" => Some(PreflightFeature::InputContainsShellCommands),
        "InputContainsCodeBlock" => Some(PreflightFeature::InputContainsCodeBlock),
        "InputContainsURL" => Some(PreflightFeature::InputContainsURL),
        "InputContainsBase64" => Some(PreflightFeature::InputContainsBase64),
        s if s.starts_with("InputLengthExceedsThreshold(") => {
            let num_str = s
                .strip_prefix("InputLengthExceedsThreshold(")?
                .strip_suffix(')')?;
            let threshold: usize = num_str.parse().ok()?;
            Some(PreflightFeature::InputLengthExceedsThreshold(threshold))
        }
        s if s.starts_with("Custom(") => {
            let pattern = s.strip_prefix("Custom(")?.strip_suffix(')')?;
            Some(PreflightFeature::Custom(pattern.to_string()))
        }
        _ => None,
    }
}

/// Build a NOT circuit: output = NOT(input)
/// When feature is true (detected), output is false (block)
fn build_block_circuit(num_inputs: usize) -> Graph {
    if num_inputs == 1 {
        // Simple NOT: NOR(x, x) = NOT(x)
        Graph {
            inputs: vec![0],
            nodes: vec![Node::new(1, BoolOp::Nor, Source::Id(0), Source::Id(0))],
            outputs: vec![1],
        }
    } else {
        // OR all inputs, then NOT: block if ANY feature is detected
        // First OR all inputs together, then NOR with itself
        let mut nodes = Vec::new();
        let mut current_id = num_inputs as u16;

        // OR chain: ((input0 OR input1) OR input2) OR ...
        let mut prev = Source::Id(0);
        for i in 1..num_inputs {
            current_id += 1;
            nodes.push(Node::new(
                current_id,
                BoolOp::Or,
                prev,
                Source::Id(i as u16),
            ));
            prev = Source::Id(current_id);
        }

        // Final NOT (NOR with itself)
        current_id += 1;
        nodes.push(Node::new(current_id, BoolOp::Nor, prev, prev));

        Graph {
            inputs: (0..num_inputs as u16).collect(),
            nodes,
            outputs: vec![current_id],
        }
    }
}

/// Build an allow circuit: output = input (pass-through)
/// When feature is true, output is true (allow)
/// For multiple inputs, uses OR - allows if ANY feature matches
fn build_allow_circuit(num_inputs: usize) -> Graph {
    if num_inputs == 1 {
        // Pass-through: output = input
        Graph {
            inputs: vec![0],
            nodes: vec![],
            outputs: vec![0],
        }
    } else {
        // OR all inputs: allow if ANY feature matches
        let mut nodes = Vec::new();
        let mut current_id = num_inputs as u16;

        let mut prev = Source::Id(0);
        for i in 1..num_inputs {
            current_id += 1;
            nodes.push(Node::new(
                current_id,
                BoolOp::Or,
                prev,
                Source::Id(i as u16),
            ));
            prev = Source::Id(current_id);
        }

        Graph {
            inputs: (0..num_inputs as u16).collect(),
            nodes,
            outputs: vec![current_id],
        }
    }
}

/// Process agent configuration (preflight, budget, KAS).
///
/// Loaded from SwarmKit `runtime` via [`load_agent_config`]. Field names keep
/// historical serde shapes for tests and migrate tooling.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    /// Agent / kit name
    #[serde(default)]
    pub name: Option<String>,
    /// Preflight policies
    #[serde(default)]
    pub preflight: Option<PreflightConfig>,
    /// Budget enforcement
    #[serde(default)]
    pub budget: Option<BudgetYamlConfig>,
    /// KAS (Key Access Service) for TDF encryption
    #[serde(default)]
    pub kas: Option<KasYamlConfig>,
}

/// Budget / spend section (kit `runtime` or legacy YAML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetYamlConfig {
    /// Maximum token cost per session in dollars
    pub max_cost_per_session: Option<f64>,
    /// Maximum token cost per day in dollars
    pub max_cost_per_day: Option<f64>,
    /// Cloud spend posture: `local_only` | `ask_before_cloud` | `cloud_within_cap`.
    /// Omitted → the safe default `ask_before_cloud`.
    #[serde(default)]
    pub cloud_policy: Option<String>,
}

/// KAS section for TDF encryption enablement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KasYamlConfig {
    /// Whether KAS is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Key identifier
    pub key_id: Option<String>,
    /// Cryptographic algorithm (e.g., "ec:secp256r1")
    pub algorithm: Option<String>,
}

/// Preflight section
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PreflightConfig {
    /// List of policies
    #[serde(default)]
    pub policies: Vec<PolicyConfig>,
}

/// Load preflight policies from the discovered SwarmKit (or empty).
pub fn load_policies_from_config() -> Result<PreflightModerator, Box<dyn std::error::Error>> {
    let config = load_agent_config()?;
    match config.preflight {
        Some(preflight) => Ok(build_moderator_from_config(&preflight)),
        None => {
            tracing::debug!("No preflight policies in SwarmKit runtime; empty moderator");
            Ok(PreflightModerator::new())
        }
    }
}

/// Load full agent config from the discovered SwarmKit kit.
///
/// Discovery order: `ARKAVO_SWARMKIT_PATH`, `.arkavo/*.swarmkit.yaml`, cwd kits.
/// Product AGENTS.md is **not** loaded. If only AGENTS.md is present, logs a
/// migrate error and returns defaults so process boot is non-fatal.
pub fn load_agent_config() -> Result<AgentConfig, Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    match arkavo_swarmkit::load_discovered_kit(&cwd) {
        Ok(discovered) => {
            let config = agent_config_from_runtime(&discovered.config);
            tracing::info!(
                path = %discovered.path.display(),
                kit = %discovered.config.kit_name,
                has_preflight = config.preflight.is_some(),
                has_budget = config.budget.is_some(),
                has_kas = config.kas.is_some(),
                "Loaded agent config from SwarmKit"
            );
            Ok(config)
        }
        Err(arkavo_swarmkit::DiscoverError::NotFound) => {
            tracing::debug!("No SwarmKit found, using default agent config");
            Ok(AgentConfig::default())
        }
        Err(arkavo_swarmkit::DiscoverError::AgentsMdUnsupported { path }) => {
            tracing::error!(
                path = %path.display(),
                "AGENTS.md is no longer supported as agent configuration. \
                 Convert with: arkavo kit migrate-from-agents-md --in {} --out agent.swarmkit.yaml",
                path.display()
            );
            Ok(AgentConfig::default())
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "SwarmKit discovery failed; using default agent config"
            );
            Ok(AgentConfig::default())
        }
    }
}

/// Map SwarmKit [`AgentRuntimeConfig`] into the router [`AgentConfig`] DTO.
pub fn agent_config_from_runtime(runtime_cfg: &arkavo_swarmkit::AgentRuntimeConfig) -> AgentConfig {
    use arkavo_swarmkit::{CloudPolicyKind, PreflightAction};

    let rt = &runtime_cfg.runtime;
    let cloud_policy = match rt.cloud_policy_or_default() {
        CloudPolicyKind::LocalOnly => "local_only",
        CloudPolicyKind::AskBeforeCloud => "ask_before_cloud",
        CloudPolicyKind::CloudWithinCap => "cloud_within_cap",
    };

    let preflight = rt.preflight.as_ref().map(|pf| PreflightConfig {
        policies: pf
            .policies
            .iter()
            .map(|p| PolicyConfig {
                id: p.id.clone(),
                features: p.features.clone(),
                action: match p.action {
                    PreflightAction::Block => PolicyAction::Block,
                    PreflightAction::Allow => PolicyAction::Allow,
                },
                description: p.description.clone(),
                enabled: p.enabled,
            })
            .collect(),
    });

    // Parity with the pre-SwarmKit behavior: a kit that says nothing about
    // spend (no session/day cap, no explicit cloud policy) must produce the
    // same `budget: None` as legacy AGENTS.md without a budget section, or
    // zero-config with no kit at all — not a BudgetManager silently built
    // from defaults. `rt.cloud_policy` (the raw Option) is the predicate
    // here, not `cloud_policy_or_default()`'s always-Some result; setting
    // cloud_policy alone is treated as an explicit spend declaration.
    let kit_declares_spend = rt.max_cost_per_session.is_some()
        || rt.max_cost_per_day.is_some()
        || rt.cloud_policy.is_some();
    let budget = kit_declares_spend.then(|| BudgetYamlConfig {
        max_cost_per_session: rt.max_cost_per_session,
        max_cost_per_day: rt.max_cost_per_day,
        cloud_policy: Some(cloud_policy.to_string()),
    });

    let kas = rt.kas.as_ref().map(|k| KasYamlConfig {
        enabled: k.enabled,
        key_id: k.key_id.clone(),
        algorithm: k.algorithm.clone(),
    });

    AgentConfig {
        name: Some(runtime_cfg.kit_name.clone()),
        preflight,
        budget,
        kas,
    }
}

/// Build a PreflightModerator from a PreflightConfig
pub fn build_moderator_from_config(config: &PreflightConfig) -> PreflightModerator {
    let moderator = PreflightModerator::new();

    for policy in &config.policies {
        if !policy.enabled {
            tracing::debug!(policy_id = %policy.id, "Skipping disabled policy");
            continue;
        }

        let features: Vec<PreflightFeature> = policy
            .features
            .iter()
            .filter_map(|f| {
                let parsed = parse_feature(f);
                if parsed.is_none() {
                    tracing::warn!(
                        policy_id = %policy.id,
                        feature = %f,
                        "Unknown feature, skipping"
                    );
                }
                parsed
            })
            .collect();

        if features.is_empty() {
            tracing::warn!(
                policy_id = %policy.id,
                "Policy has no valid features, skipping"
            );
            continue;
        }

        let graph = match policy.action {
            PolicyAction::Block => build_block_circuit(features.len()),
            PolicyAction::Allow => build_allow_circuit(features.len()),
        };

        moderator.register_graph(PolicyId::new(&policy.id), graph, features);

        tracing::info!(
            policy_id = %policy.id,
            action = ?policy.action,
            "Loaded preflight policy from config"
        );
    }

    moderator
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_feature() {
        assert!(matches!(
            parse_feature("InputContainsPII"),
            Some(PreflightFeature::InputContainsPII)
        ));
        assert!(matches!(
            parse_feature("InputContainsSQLKeywords"),
            Some(PreflightFeature::InputContainsSQLKeywords)
        ));
        assert!(matches!(
            parse_feature("InputLengthExceedsThreshold(1000)"),
            Some(PreflightFeature::InputLengthExceedsThreshold(1000))
        ));
        assert!(parse_feature("Unknown").is_none());
    }

    #[test]
    fn agent_config_from_swarmkit_runtime_maps_cloud_and_preflight() {
        use arkavo_swarmkit::{
            AgentRuntimeConfig, CloudPolicyKind, KitRuntimeConfig, PreflightAction,
            PreflightPolicySpec, RoleRuntimeView, RuntimePreflight,
        };

        let runtime_cfg = AgentRuntimeConfig {
            kit_id: "blake3:test".into(),
            kit_name: "secure-kit".into(),
            objective_goal: "stay safe".into(),
            runtime: KitRuntimeConfig {
                cloud_policy: Some(CloudPolicyKind::LocalOnly),
                max_cost_per_session: Some(1.0),
                preflight: Some(RuntimePreflight {
                    policies: vec![PreflightPolicySpec {
                        id: "block_pii".into(),
                        features: vec!["InputContainsPII".into()],
                        action: PreflightAction::Block,
                        description: None,
                        enabled: true,
                    }],
                }),
                ..Default::default()
            },
            roles: vec![RoleRuntimeView {
                role_id: "agent".into(),
                role_type: "operator".into(),
                description: None,
                model_family: Some("ministral".into()),
                model_size: Some("3B".into()),
                model_backend: None,
                skill_instructions: String::new(),
                mcp_tool_grants: vec![],
            }],
        };

        let config = agent_config_from_runtime(&runtime_cfg);
        assert_eq!(config.name.as_deref(), Some("secure-kit"));
        assert_eq!(
            config.budget.as_ref().unwrap().cloud_policy.as_deref(),
            Some("local_only")
        );
        let moderator = build_moderator_from_config(config.preflight.as_ref().unwrap());
        assert_eq!(moderator.len(), 1);
        assert!(moderator.check("SSN 123-45-6789").is_blocked());
    }

    #[test]
    fn agent_config_from_swarmkit_runtime_without_runtime_block_has_no_budget() {
        use arkavo_swarmkit::{AgentRuntimeConfig, KitRuntimeConfig, RoleRuntimeView};

        let runtime_cfg = AgentRuntimeConfig {
            kit_id: "blake3:test".into(),
            kit_name: "no-runtime-kit".into(),
            objective_goal: "stay safe".into(),
            runtime: KitRuntimeConfig::default(),
            roles: vec![RoleRuntimeView {
                role_id: "agent".into(),
                role_type: "operator".into(),
                description: None,
                model_family: None,
                model_size: None,
                model_backend: None,
                skill_instructions: String::new(),
                mcp_tool_grants: vec![],
            }],
        };

        let config = agent_config_from_runtime(&runtime_cfg);
        assert!(
            config.budget.is_none(),
            "a kit with no runtime block must not produce a budget, matching \
             legacy AGENTS.md-without-budget and zero-config parity"
        );
    }

    #[test]
    fn agent_config_from_swarmkit_runtime_with_runtime_but_no_cost_fields_has_no_budget() {
        use arkavo_swarmkit::{AgentRuntimeConfig, KitRuntimeConfig, RoleRuntimeView, RuntimeMode};

        let runtime_cfg = AgentRuntimeConfig {
            kit_id: "blake3:test".into(),
            kit_name: "runtime-no-cost-kit".into(),
            objective_goal: "stay safe".into(),
            runtime: KitRuntimeConfig {
                mode: Some(RuntimeMode::Specialist),
                mdns: Some(false),
                ..Default::default()
            },
            roles: vec![RoleRuntimeView {
                role_id: "agent".into(),
                role_type: "operator".into(),
                description: None,
                model_family: None,
                model_size: None,
                model_backend: None,
                skill_instructions: String::new(),
                mcp_tool_grants: vec![],
            }],
        };

        let config = agent_config_from_runtime(&runtime_cfg);
        assert!(
            config.budget.is_none(),
            "a runtime block that declares no cost fields must still yield no budget"
        );
    }

    #[test]
    fn agent_config_from_swarmkit_runtime_with_session_cost_has_budget() {
        use arkavo_swarmkit::{AgentRuntimeConfig, KitRuntimeConfig, RoleRuntimeView};

        let runtime_cfg = AgentRuntimeConfig {
            kit_id: "blake3:test".into(),
            kit_name: "spend-kit".into(),
            objective_goal: "stay safe".into(),
            runtime: KitRuntimeConfig {
                max_cost_per_session: Some(2.5),
                ..Default::default()
            },
            roles: vec![RoleRuntimeView {
                role_id: "agent".into(),
                role_type: "operator".into(),
                description: None,
                model_family: None,
                model_size: None,
                model_backend: None,
                skill_instructions: String::new(),
                mcp_tool_grants: vec![],
            }],
        };

        let config = agent_config_from_runtime(&runtime_cfg);
        let budget = config
            .budget
            .expect("max_cost_per_session must yield a budget");
        assert_eq!(budget.max_cost_per_session, Some(2.5));
    }

    #[test]
    fn agent_config_from_swarmkit_runtime_maps_kas() {
        use arkavo_swarmkit::{AgentRuntimeConfig, KitRuntimeConfig, RoleRuntimeView, RuntimeKas};

        let runtime_cfg = AgentRuntimeConfig {
            kit_id: "blake3:test".into(),
            kit_name: "encrypted-kit".into(),
            objective_goal: "stay safe".into(),
            runtime: KitRuntimeConfig {
                kas: Some(RuntimeKas {
                    enabled: true,
                    key_id: Some("my-key-42".into()),
                    algorithm: Some("ec:secp256r1".into()),
                    trusted_roots: vec![],
                }),
                ..Default::default()
            },
            roles: vec![RoleRuntimeView {
                role_id: "agent".into(),
                role_type: "operator".into(),
                description: None,
                model_family: None,
                model_size: None,
                model_backend: None,
                skill_instructions: String::new(),
                mcp_tool_grants: vec![],
            }],
        };

        let config = agent_config_from_runtime(&runtime_cfg);
        let kas = config.kas.unwrap();
        assert!(kas.enabled);
        assert_eq!(kas.key_id.as_deref(), Some("my-key-42"));
        assert_eq!(kas.algorithm.as_deref(), Some("ec:secp256r1"));
    }

    #[test]
    fn agent_config_from_swarmkit_runtime_kas_disabled_by_default() {
        use arkavo_swarmkit::{AgentRuntimeConfig, KitRuntimeConfig, RoleRuntimeView, RuntimeKas};

        let runtime_cfg = AgentRuntimeConfig {
            kit_id: "blake3:test".into(),
            kit_name: "minimal-kas-kit".into(),
            objective_goal: "stay safe".into(),
            runtime: KitRuntimeConfig {
                kas: Some(RuntimeKas {
                    enabled: false,
                    key_id: None,
                    algorithm: None,
                    trusted_roots: vec![],
                }),
                ..Default::default()
            },
            roles: vec![RoleRuntimeView {
                role_id: "agent".into(),
                role_type: "operator".into(),
                description: None,
                model_family: None,
                model_size: None,
                model_backend: None,
                skill_instructions: String::new(),
                mcp_tool_grants: vec![],
            }],
        };

        let config = agent_config_from_runtime(&runtime_cfg);
        let kas = config.kas.unwrap();
        assert!(!kas.enabled);
        assert!(kas.key_id.is_none());
    }

    #[test]
    fn test_build_moderator_from_config() {
        let config = PreflightConfig {
            policies: vec![
                PolicyConfig {
                    id: "block_pii".to_string(),
                    features: vec!["InputContainsPII".to_string()],
                    action: PolicyAction::Block,
                    description: None,
                    enabled: true,
                },
                PolicyConfig {
                    id: "disabled".to_string(),
                    features: vec!["InputContainsURL".to_string()],
                    action: PolicyAction::Block,
                    description: None,
                    enabled: false,
                },
            ],
        };

        let moderator = build_moderator_from_config(&config);
        assert_eq!(moderator.len(), 1);

        let result = moderator.check("My SSN is 123-45-6789");
        assert!(result.is_blocked());
    }
}
