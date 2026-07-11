//! Kit-level `runtime` block — process/network/spend/preflight config that
//! previously lived in AGENTS.md. Optional: existing kits omit it and keep
//! the same `kit.id` (field is skipped when absent).

use serde::{Deserialize, Serialize};

use crate::manifest::Manifest;
use crate::role::RoleSpec;
use crate::skill_content::SkillContent;

/// How the agent process behaves when this kit is the primary config.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    /// Autonomous tick / orchestrator loop.
    #[default]
    Orchestrator,
    /// Passive: only responds to delegated tasks.
    Specialist,
}

/// Cloud spend posture (spend plane). Same vocabulary as arkavo-budget.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloudPolicyKind {
    LocalOnly,
    #[default]
    AskBeforeCloud,
    CloudWithinCap,
}

/// Preflight action for a policy entry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PreflightAction {
    #[default]
    Block,
    Allow,
}

/// Single preflight policy (mirrors historical AGENTS.md frontmatter shape).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreflightPolicySpec {
    pub id: String,
    pub features: Vec<String>,
    #[serde(default)]
    pub action: PreflightAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Preflight section under `runtime`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RuntimePreflight {
    #[serde(default)]
    pub policies: Vec<PreflightPolicySpec>,
}

/// KAS enablement for TDF audit paths (key material stays out of the kit).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RuntimeKas {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<String>,
    /// DID-identified authorities this kit trusts as TDF attribute issuers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_roots: Vec<TrustedRoot>,
}

/// A single trusted KAS root, identified by DID.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrustedRoot {
    pub did: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// MCP server process/URL metadata for tool bridges.
/// Grants remain on roles via `mcp_tools`; this block supplies how to reach a server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeMcpServer {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Kit-level runtime configuration replacing product AGENTS.md fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct KitRuntimeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<RuntimeMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mdns: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_policy: Option<CloudPolicyKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_per_session: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_per_day: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kas: Option<RuntimeKas>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preflight: Option<RuntimePreflight>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<RuntimeMcpServer>,
    /// When true, skill signature verification may be skipped for local authoring.
    /// Production flights should leave this unset/false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_dev: Option<bool>,
}

impl KitRuntimeConfig {
    /// Effective process mode (default orchestrator).
    pub fn mode_or_default(&self) -> RuntimeMode {
        self.mode.unwrap_or_default()
    }

    /// Effective cloud policy (default ask_before_cloud).
    pub fn cloud_policy_or_default(&self) -> CloudPolicyKind {
        self.cloud_policy.unwrap_or_default()
    }

    /// Whether mDNS is enabled (default true for zero-config).
    pub fn mdns_or_default(&self) -> bool {
        self.mdns.unwrap_or(true)
    }

    /// Local authoring kit (unsigned skills allowed by consumers that honor this).
    pub fn is_local_dev(&self) -> bool {
        self.local_dev.unwrap_or(false)
    }
}

/// Resolved view for a single role used by process bootstrap.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleRuntimeView {
    pub role_id: String,
    pub role_type: String,
    pub description: Option<String>,
    pub model_family: Option<String>,
    pub model_size: Option<String>,
    pub model_backend: Option<String>,
    /// Concatenated skill instructions (identity / operating prompt).
    pub skill_instructions: String,
    /// Flattened tool names from role `mcp_tools` grants.
    pub mcp_tool_grants: Vec<String>,
}

/// Process-facing config derived from a validated Manifest.
/// Replaces dual AGENTS.md parsers for identity, spend, preflight, and roles.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRuntimeConfig {
    pub kit_id: String,
    pub kit_name: String,
    pub objective_goal: String,
    pub runtime: KitRuntimeConfig,
    pub roles: Vec<RoleRuntimeView>,
}

impl AgentRuntimeConfig {
    /// Primary role for single-agent kits: first role.
    pub fn primary_role(&self) -> Option<&RoleRuntimeView> {
        self.roles.first()
    }

    /// System-style purpose string: objective + primary skill instructions.
    pub fn purpose_text(&self) -> String {
        let mut parts = vec![self.objective_goal.clone()];
        if let Some(role) = self.primary_role()
            && !role.skill_instructions.is_empty()
        {
            parts.push(role.skill_instructions.clone());
        }
        parts.join("\n\n")
    }
}

/// Build [`AgentRuntimeConfig`] from a validated manifest.
pub fn agent_runtime_config_from_manifest(m: &Manifest) -> AgentRuntimeConfig {
    let runtime = m.runtime.clone().unwrap_or_default();
    let roles = m.roles.iter().map(role_runtime_view).collect();

    AgentRuntimeConfig {
        kit_id: m.kit.id.clone(),
        kit_name: m.kit.name.clone(),
        objective_goal: m.objective.goal.clone(),
        runtime,
        roles,
    }
}

fn role_runtime_view(r: &RoleSpec) -> RoleRuntimeView {
    let model = r.agent_provisioning.model.as_ref();
    let mcp_tool_grants = r
        .mcp_tools
        .iter()
        .flat_map(|g| g.tools.iter().cloned())
        .collect();

    RoleRuntimeView {
        role_id: r.id.clone(),
        role_type: r.role_type.clone(),
        description: r.description.clone(),
        model_family: model.map(|m| m.family.clone()),
        model_size: model.and_then(|m| m.size.clone()),
        model_backend: model.and_then(|m| m.backend.clone()),
        skill_instructions: collect_skill_instructions(r),
        mcp_tool_grants,
    }
}

fn collect_skill_instructions(role: &RoleSpec) -> String {
    let mut parts = Vec::new();
    for skill in &role.skills {
        let Some(payload) = &skill.payload else {
            continue;
        };
        if let Ok(content) = serde_json::from_value::<SkillContent>(payload.clone()) {
            if !content.instructions.is_empty() {
                parts.push(content.instructions);
            }
            continue;
        }
        if let Some(instr) = payload.get("instructions").and_then(|v| v.as_str()) {
            parts.push(instr.to_string());
        }
    }
    parts.join("\n\n")
}

/// Validate kit-level runtime block (called from cross-block validate).
pub fn validate_runtime(runtime: &KitRuntimeConfig) -> Result<(), RuntimeValidationError> {
    if let Some(preflight) = &runtime.preflight {
        let mut seen = std::collections::HashSet::new();
        for policy in &preflight.policies {
            if policy.id.is_empty() {
                return Err(RuntimeValidationError::EmptyPreflightPolicyId);
            }
            if !seen.insert(policy.id.clone()) {
                return Err(RuntimeValidationError::DuplicatePreflightPolicyId(
                    policy.id.clone(),
                ));
            }
            if policy.features.is_empty() {
                return Err(RuntimeValidationError::EmptyPreflightFeatures {
                    policy_id: policy.id.clone(),
                });
            }
        }
    }

    if let Some(cost) = runtime.max_cost_per_session
        && cost < 0.0
    {
        return Err(RuntimeValidationError::NegativeCost {
            field: "max_cost_per_session",
        });
    }
    if let Some(cost) = runtime.max_cost_per_day
        && cost < 0.0
    {
        return Err(RuntimeValidationError::NegativeCost {
            field: "max_cost_per_day",
        });
    }

    if let Some(kas) = &runtime.kas {
        for root in &kas.trusted_roots {
            if !root.did.starts_with("did:") {
                return Err(RuntimeValidationError::InvalidTrustedRootDid(
                    root.did.clone(),
                ));
            }
        }
    }

    let mut server_names = std::collections::HashSet::new();
    for server in &runtime.mcp_servers {
        if server.name.is_empty() {
            return Err(RuntimeValidationError::EmptyMcpServerName);
        }
        if !server_names.insert(server.name.clone()) {
            return Err(RuntimeValidationError::DuplicateMcpServerName(
                server.name.clone(),
            ));
        }
        if server.command.is_none() && server.url.is_none() {
            return Err(RuntimeValidationError::McpServerMissingEndpoint {
                name: server.name.clone(),
            });
        }
    }

    Ok(())
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RuntimeValidationError {
    #[error("runtime.preflight.policies entry has empty id")]
    EmptyPreflightPolicyId,

    #[error("runtime.preflight.policies duplicate id {0:?}")]
    DuplicatePreflightPolicyId(String),

    #[error("runtime.preflight.policies {policy_id:?} has empty features")]
    EmptyPreflightFeatures { policy_id: String },

    #[error("runtime.{field} must not be negative")]
    NegativeCost { field: &'static str },

    #[error("runtime.mcp_servers entry has empty name")]
    EmptyMcpServerName,

    #[error("runtime.mcp_servers duplicate name {0:?}")]
    DuplicateMcpServerName(String),

    #[error("runtime.mcp_servers {name:?} needs command or url")]
    McpServerMissingEndpoint { name: String },

    #[error("runtime.kas.trusted_roots did {0:?} must be non-empty and start with \"did:\"")]
    InvalidTrustedRootDid(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[test]
    fn runtime_yaml_round_trip() {
        let yaml = r#"
mode: specialist
listen: "127.0.0.1:8787"
mdns: false
cloud_policy: local_only
max_cost_per_session: 1.5
local_dev: true
preflight:
  policies:
    - id: no-pii
      features: ["InputContainsPII"]
      action: block
      enabled: true
mcp_servers:
  - name: fs
    command: arkavo
    args: ["mcp", "filesystem"]
"#;
        let cfg: KitRuntimeConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.mode, Some(RuntimeMode::Specialist));
        assert_eq!(cfg.cloud_policy, Some(CloudPolicyKind::LocalOnly));
        assert_eq!(cfg.mdns, Some(false));
        assert!(cfg.is_local_dev());
        assert_eq!(cfg.preflight.as_ref().unwrap().policies.len(), 1);
        assert_eq!(cfg.mcp_servers[0].name, "fs");
        validate_runtime(&cfg).unwrap();
    }

    #[test]
    fn defaults_when_empty() {
        let cfg: KitRuntimeConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.mode_or_default(), RuntimeMode::Orchestrator);
        assert_eq!(
            cfg.cloud_policy_or_default(),
            CloudPolicyKind::AskBeforeCloud
        );
        assert!(cfg.mdns_or_default());
        assert!(!cfg.is_local_dev());
        validate_runtime(&cfg).unwrap();
    }

    #[test]
    fn rejects_unknown_cloud_policy() {
        let err = serde_yaml::from_str::<KitRuntimeConfig>("cloud_policy: spend_everything");
        assert!(err.is_err());
    }

    #[spec("SK-100")]
    #[test]
    fn duplicate_preflight_policy_id_rejected() {
        let cfg = KitRuntimeConfig {
            preflight: Some(RuntimePreflight {
                policies: vec![
                    PreflightPolicySpec {
                        id: "p1".into(),
                        features: vec!["InputContainsPII".into()],
                        action: PreflightAction::Block,
                        description: None,
                        enabled: true,
                    },
                    PreflightPolicySpec {
                        id: "p1".into(),
                        features: vec!["InputContainsURL".into()],
                        action: PreflightAction::Block,
                        description: None,
                        enabled: true,
                    },
                ],
            }),
            ..Default::default()
        };
        let err = validate_runtime(&cfg).unwrap_err();
        assert!(matches!(
            err,
            RuntimeValidationError::DuplicatePreflightPolicyId(_)
        ));
    }

    #[spec("SK-100")]
    #[test]
    fn mcp_server_requires_endpoint() {
        let cfg = KitRuntimeConfig {
            mcp_servers: vec![RuntimeMcpServer {
                name: "orphan".into(),
                command: None,
                args: vec![],
                url: None,
            }],
            ..Default::default()
        };
        assert!(matches!(
            validate_runtime(&cfg),
            Err(RuntimeValidationError::McpServerMissingEndpoint { .. })
        ));
    }

    #[spec("SK-103")]
    #[test]
    fn kas_trusted_roots_parse_and_validate() {
        let yaml = r#"
kas:
  enabled: true
  key_id: "kas-key-1"
  algorithm: "ec:secp256r1"
  trusted_roots:
    - did: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
      name: "Demo Root Authority"
"#;
        let cfg: KitRuntimeConfig = serde_yaml::from_str(yaml).unwrap();
        let kas = cfg.kas.as_ref().expect("kas block present");
        assert_eq!(kas.trusted_roots.len(), 1);
        assert_eq!(
            kas.trusted_roots[0].did,
            "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
        );
        assert_eq!(
            kas.trusted_roots[0].name.as_deref(),
            Some("Demo Root Authority")
        );
        validate_runtime(&cfg).unwrap();
    }

    #[spec("SK-103")]
    #[test]
    fn kas_trusted_root_rejects_malformed_did() {
        let cfg = KitRuntimeConfig {
            kas: Some(RuntimeKas {
                enabled: true,
                trusted_roots: vec![TrustedRoot {
                    did: "not-a-did".into(),
                    name: None,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(matches!(
            validate_runtime(&cfg),
            Err(RuntimeValidationError::InvalidTrustedRootDid(_))
        ));
    }

    #[spec("SK-103")]
    #[test]
    fn kas_trusted_root_rejects_empty_did() {
        let cfg = KitRuntimeConfig {
            kas: Some(RuntimeKas {
                enabled: true,
                trusted_roots: vec![TrustedRoot {
                    did: String::new(),
                    name: None,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(matches!(
            validate_runtime(&cfg),
            Err(RuntimeValidationError::InvalidTrustedRootDid(_))
        ));
    }

    #[spec("SK-103")]
    #[test]
    fn kas_without_trusted_roots_still_validates() {
        let cfg: KitRuntimeConfig = serde_yaml::from_str("kas:\n  enabled: true\n").unwrap();
        assert!(cfg.kas.as_ref().unwrap().trusted_roots.is_empty());
        validate_runtime(&cfg).unwrap();
    }

    #[spec("SK-103")]
    #[test]
    fn kas_trusted_roots_absent_when_empty_on_round_trip() {
        let cfg = KitRuntimeConfig {
            kas: Some(RuntimeKas {
                enabled: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        assert!(
            !yaml.contains("trusted_roots"),
            "empty trusted_roots must be omitted from serialized output: {yaml}"
        );
    }
}
