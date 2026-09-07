//! Runtime policy configuration for preflight moderation
//!
//! Policies are loaded from agent AGENTS.md files at runtime, not hardcoded.
//! Each agent defines its own preflight policies in its configuration.

use super::features::PreflightFeature;
use super::moderator::{PolicyId, PreflightModerator};
use serde::{Deserialize, Serialize};
use std::path::Path;
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

/// AGENTS.md configuration with preflight, budget, and KAS policies
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    /// Agent name
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

/// Budget section in AGENTS.md YAML frontmatter
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

/// KAS section in AGENTS.md YAML frontmatter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KasYamlConfig {
    /// Whether KAS is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Key identifier
    pub key_id: Option<String>,
    /// Cryptographic algorithm (e.g., "ec:secp256r1")
    pub algorithm: Option<String>,
    /// Trusted root authorities for delegation chain verification.
    /// A `kas.rewrap` delegation chain must terminate at one of these DIDs.
    #[serde(default)]
    pub trusted_roots: Vec<KasTrustedRootYaml>,
}

/// A trusted root authority entry in the KAS YAML config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KasTrustedRootYaml {
    /// DID:key identifier of the root authority
    pub did: String,
    /// Optional human-readable label
    #[serde(default)]
    pub name: Option<String>,
    /// Optional Ed25519 public key (base64). The verifying key is also
    /// recoverable from the DID:key itself; this field documents intent.
    #[serde(default)]
    pub public_key: Option<String>,
}

/// Preflight section in AGENTS.md
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PreflightConfig {
    /// List of policies
    #[serde(default)]
    pub policies: Vec<PolicyConfig>,
}

/// Load policies from an agent's AGENTS.md file
///
/// Searches for AGENTS.md in the current directory and parent directories.
/// Returns an empty moderator if no AGENTS.md is found or no policies defined.
pub fn load_policies_from_config() -> Result<PreflightModerator, Box<dyn std::error::Error>> {
    // Search for AGENTS.md starting from current directory
    let mut current = std::env::current_dir()?;

    loop {
        let agents_md = current.join("AGENTS.md");
        if agents_md.exists() {
            return load_policies_from_agents_md(&agents_md);
        }

        if !current.pop() {
            break;
        }
    }

    tracing::debug!("No AGENTS.md found, using empty moderator");
    Ok(PreflightModerator::new())
}

/// Load policies from a specific AGENTS.md file
pub fn load_policies_from_agents_md(
    path: &Path,
) -> Result<PreflightModerator, Box<dyn std::error::Error>> {
    let moderator = PreflightModerator::new();

    if !path.exists() {
        tracing::debug!(
            path = %path.display(),
            "AGENTS.md not found, using empty moderator"
        );
        return Ok(moderator);
    }

    let content = std::fs::read_to_string(path)?;

    // Extract YAML content from markdown (between --- markers or parse directly)
    let yaml_content = extract_yaml_from_markdown(&content);

    let config: AgentConfig = serde_yaml::from_str(&yaml_content).unwrap_or_default();

    let policies = match config.preflight {
        Some(preflight) => preflight.policies,
        None => {
            tracing::debug!(
                path = %path.display(),
                "No preflight policies in AGENTS.md"
            );
            return Ok(moderator);
        }
    };

    for policy in policies {
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
            "Loaded preflight policy from AGENTS.md"
        );
    }

    tracing::info!(
        policies_loaded = moderator.len(),
        path = %path.display(),
        "Preflight policies loaded from AGENTS.md"
    );

    Ok(moderator)
}

/// Load full agent config from AGENTS.md (includes preflight, budget, KAS)
///
/// Searches for AGENTS.md in the current directory and parent directories.
/// Returns default config if no AGENTS.md is found.
pub fn load_agent_config() -> Result<AgentConfig, Box<dyn std::error::Error>> {
    let mut current = std::env::current_dir()?;

    loop {
        let agents_md = current.join("AGENTS.md");
        if agents_md.exists() {
            return load_agent_config_from_agents_md(&agents_md);
        }

        if !current.pop() {
            break;
        }
    }

    tracing::debug!("No AGENTS.md found, using default agent config");
    Ok(AgentConfig::default())
}

/// Load full agent config from a specific AGENTS.md file
pub fn load_agent_config_from_agents_md(
    path: &Path,
) -> Result<AgentConfig, Box<dyn std::error::Error>> {
    if !path.exists() {
        tracing::debug!(
            path = %path.display(),
            "AGENTS.md not found, using default agent config"
        );
        return Ok(AgentConfig::default());
    }

    let content = std::fs::read_to_string(path)?;
    let yaml_content = extract_yaml_from_markdown(&content);
    let config: AgentConfig = serde_yaml::from_str(&yaml_content).unwrap_or_default();

    tracing::info!(
        path = %path.display(),
        has_preflight = config.preflight.is_some(),
        has_budget = config.budget.is_some(),
        has_kas = config.kas.is_some(),
        "Loaded agent config from AGENTS.md"
    );

    Ok(config)
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

/// Extract YAML content from markdown file
/// Handles both frontmatter (---) and inline YAML sections
fn extract_yaml_from_markdown(content: &str) -> String {
    // Try frontmatter first (between --- markers)
    if let Some(after_start) = content.strip_prefix("---")
        && let Some(end) = after_start.find("---")
    {
        return after_start[..end].to_string();
    }

    // Otherwise, try to parse the whole content as YAML-like markdown
    // Extract key: value pairs from the markdown
    let mut yaml_lines = Vec::new();
    let mut in_yaml_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip markdown headers and comments
        if trimmed.starts_with('#') && !trimmed.starts_with("# ") {
            continue;
        }
        if trimmed.starts_with("## ") {
            continue;
        }

        // Detect YAML-like content
        if trimmed.contains(':') && !trimmed.starts_with("```") {
            yaml_lines.push(line.to_string());
            in_yaml_block = trimmed.ends_with(':') || trimmed.ends_with(": |");
        } else if in_yaml_block && (trimmed.starts_with('-') || trimmed.starts_with(' ')) {
            yaml_lines.push(line.to_string());
        } else {
            in_yaml_block = false;
        }
    }

    yaml_lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;

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
    fn test_load_agents_md_with_preflight() {
        let mut file = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(
            file,
            r#"---
name: test-agent
preflight:
  policies:
    - id: block_pii
      features:
        - InputContainsPII
      action: block
      enabled: true
---
"#
        )
        .unwrap();

        let path = file.path().to_path_buf();
        let moderator = load_policies_from_agents_md(&path).unwrap();
        assert_eq!(moderator.len(), 1);

        // Test that it blocks PII
        let result = moderator.check("My SSN is 123-45-6789");
        assert!(result.is_blocked());
    }

    #[test]
    fn test_load_agents_md_multiple_policies() {
        let mut file = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(
            file,
            r#"---
name: secure-agent
preflight:
  policies:
    - id: block_pii
      features:
        - InputContainsPII
      action: block
    - id: block_sql
      features:
        - InputContainsSQLKeywords
      action: block
    - id: disabled_policy
      features:
        - InputContainsShellCommands
      action: block
      enabled: false
---
"#
        )
        .unwrap();

        let path = file.path().to_path_buf();
        let moderator = load_policies_from_agents_md(&path).unwrap();
        // Only 2 policies loaded (one is disabled)
        assert_eq!(moderator.len(), 2);
    }

    #[test]
    fn test_missing_agents_md_returns_empty() {
        let path = PathBuf::from("/nonexistent/path/AGENTS.md");
        let moderator = load_policies_from_agents_md(&path).unwrap();
        assert!(moderator.is_empty());
    }

    #[test]
    fn test_load_agents_md_with_budget() {
        let mut file = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(
            file,
            r#"---
name: budget-agent
budget:
  max_cost_per_session: 5.0
  max_cost_per_day: 25.0
---
"#
        )
        .unwrap();

        let path = file.path().to_path_buf();
        let config = load_agent_config_from_agents_md(&path).unwrap();
        assert_eq!(config.name.as_deref(), Some("budget-agent"));
        let budget = config.budget.unwrap();
        assert!((budget.max_cost_per_session.unwrap() - 5.0).abs() < f64::EPSILON);
        assert!((budget.max_cost_per_day.unwrap() - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_load_agents_md_with_kas_trusted_roots() {
        let mut file = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(
            file,
            r#"---
name: kas-root-agent
kas:
  enabled: true
  key_id: my-key-42
  algorithm: ec:secp256r1
  trusted_roots:
    - did: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
      name: "Demo Root Authority"
    - did: "did:key:z6MkSecondRoot"
      public_key: "dGVzdC1wdWJsaWMta2V5"
---
"#
        )
        .unwrap();

        let path = file.path().to_path_buf();
        let config = load_agent_config_from_agents_md(&path).unwrap();
        let kas = config.kas.unwrap();
        assert_eq!(kas.trusted_roots.len(), 2);
        assert_eq!(
            kas.trusted_roots[0].did,
            "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
        );
        assert_eq!(
            kas.trusted_roots[0].name.as_deref(),
            Some("Demo Root Authority")
        );
        assert!(kas.trusted_roots[0].public_key.is_none());
        assert_eq!(kas.trusted_roots[1].did, "did:key:z6MkSecondRoot");
        assert_eq!(
            kas.trusted_roots[1].public_key.as_deref(),
            Some("dGVzdC1wdWJsaWMta2V5")
        );
    }

    #[test]
    fn test_load_agents_md_with_kas() {
        let mut file = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(
            file,
            r#"---
name: encrypted-agent
kas:
  enabled: true
  key_id: my-key-42
  algorithm: ec:secp256r1
---
"#
        )
        .unwrap();

        let path = file.path().to_path_buf();
        let config = load_agent_config_from_agents_md(&path).unwrap();
        assert_eq!(config.name.as_deref(), Some("encrypted-agent"));
        let kas = config.kas.unwrap();
        assert!(kas.enabled);
        assert_eq!(kas.key_id.as_deref(), Some("my-key-42"));
        assert_eq!(kas.algorithm.as_deref(), Some("ec:secp256r1"));
    }

    #[test]
    fn test_load_agents_md_full_config() {
        let mut file = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(
            file,
            r#"---
name: full-agent
preflight:
  policies:
    - id: block_pii
      features:
        - InputContainsPII
      action: block
budget:
  max_cost_per_session: 10.0
  max_cost_per_day: 50.0
kas:
  enabled: true
  key_id: kas-key-1
  algorithm: ec:secp256r1
---
"#
        )
        .unwrap();

        let path = file.path().to_path_buf();
        let config = load_agent_config_from_agents_md(&path).unwrap();
        assert!(config.preflight.is_some());
        assert!(config.budget.is_some());
        assert!(config.kas.is_some());

        // Verify preflight can build a moderator
        let moderator = build_moderator_from_config(config.preflight.as_ref().unwrap());
        assert_eq!(moderator.len(), 1);
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

    #[test]
    fn test_kas_disabled_by_default() {
        let mut file = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(
            file,
            r#"---
name: minimal-kas
kas:
  enabled: false
---
"#
        )
        .unwrap();

        let path = file.path().to_path_buf();
        let config = load_agent_config_from_agents_md(&path).unwrap();
        let kas = config.kas.unwrap();
        assert!(!kas.enabled);
        assert!(kas.key_id.is_none());
    }

    #[test]
    fn test_agents_md_no_preflight_returns_empty() {
        let mut file = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(
            file,
            r#"---
name: simple-agent
model: gemma-3-270m
---
"#
        )
        .unwrap();

        let path = file.path().to_path_buf();
        let moderator = load_policies_from_agents_md(&path).unwrap();
        assert!(moderator.is_empty());
    }
}
