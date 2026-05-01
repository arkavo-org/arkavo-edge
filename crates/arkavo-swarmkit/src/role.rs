//! RoleSpec and the per-role `agent_provisioning` block per spec §4.3 and §5.

use serde::{Deserialize, Serialize};

/// RoleSpec per §4.3. `role_type` is free-form per Appendix C.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoleSpec {
    pub id: String,
    pub role_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub agent_provisioning: AgentProvisioning,
    #[serde(default)]
    pub skills: Vec<Skill>,
    #[serde(default)]
    pub mcp_tools: Vec<McpToolGrant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tdf_attribute_release_policy: Option<TdfAttributeReleasePolicy>,
    #[serde(default)]
    pub handoffs: Vec<Handoff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_scope: Option<ContextScope>,
}

/// Per-role provisioning block per spec §5.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AgentProvisioning {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Model>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inference: Option<Inference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<Budget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use: Option<ToolUse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observability: Option<Observability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<Isolation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Model {
    pub family: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<ModelFallback>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelFallback {
    pub family: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Inference {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<bool>,
    #[serde(default)]
    pub stop_sequences: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Budget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_inference_calls: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wallclock_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolUse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_calls: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallel: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_error: Option<ToolUseOnError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<RetryPolicy>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolUseOnError {
    Retry,
    Skip,
    Abort,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetryPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_context_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kv_cache_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_strategy: Option<CompactionStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistence: Option<Persistence>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStrategy {
    ToolResult,
    Summary,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Persistence {
    Ephemeral,
    Session,
    Kit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Observability {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_level: Option<LogLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics_endpoint: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Isolation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<Sandbox>,
    #[serde(default)]
    pub fs_writable: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_egress: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sandbox {
    Process,
    Container,
    Vm,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Failure {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_timeout: Option<OnTimeout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub circuit_breaker: Option<CircuitBreaker>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnTimeout {
    Retry,
    Abort,
    Fallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CircuitBreaker {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Skill {
    pub id: String,
    pub version: String,
    pub source: SkillSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SkillSource {
    Inline,
    Registry,
    TdfRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpToolGrant {
    pub server: String,
    pub tools: Vec<String>,
    pub auth: AuthMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    Delegated,
    Passthrough,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TdfAttributeReleasePolicy {
    pub attributes: Vec<String>,
    pub rule: ArpRule,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ArpRule {
    AllOf,
    AnyOf,
    Hierarchy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Handoff {
    pub to: String,
    pub on: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ContextScope {
    #[serde(default)]
    pub can_read: Vec<String>,
    #[serde(default)]
    pub can_write: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SK-006")]
    #[test]
    fn role_type_is_free_form() {
        let json = r#"{
            "id": "campaign-lead-1",
            "role_type": "campaign_lead",
            "agent_provisioning": {}
        }"#;
        let role: RoleSpec = serde_json::from_str(json).unwrap();
        assert_eq!(role.role_type, "campaign_lead");
    }

    #[test]
    fn auth_mode_serializes_snake_case() {
        let m = AuthMode::Delegated;
        assert_eq!(serde_json::to_string(&m).unwrap(), "\"delegated\"");
    }

    #[test]
    fn arp_rule_serializes_camel_case() {
        let r = ArpRule::AllOf;
        assert_eq!(serde_json::to_string(&r).unwrap(), "\"allOf\"");
    }
}
