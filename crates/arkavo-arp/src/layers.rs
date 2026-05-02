//! Layer-specific adaptation types: cognitive (§9), execution (§10),
//! data sovereignty (§11), network (§12).

use serde::{Deserialize, Serialize};

use crate::model::{Sensitivity, SensitivityMultipliers};

// --- Layer 1: Cognitive (§9) ---

/// Layer 1: Cognitive and intent adaptation (§9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cognitive {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_inspection: Option<ContextInspection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drift_detection: Option<DriftDetection>,
}

/// Context inspection and sanitization (§9.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextInspection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_registry: Option<Vec<PiiPattern>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction_profiles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_teachable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub injection_detection: Option<InjectionDetection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_input_chars: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_chars: Option<u64>,
}

/// PII detection pattern entry (§9.1.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiiPattern {
    pub name: String,
    pub regex: String,
    pub sensitivity: Sensitivity,
    pub action: PiiAction,
}

/// Action to take when PII is detected.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PiiAction {
    Block,
    Redact,
    Log,
}

/// Injection detection configuration (§9.1.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectionDetection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql_keywords: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell_commands: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64_detection_threshold: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_injection_patterns: Option<Vec<serde_json::Value>>,
}

/// Semantic drift detection (§9.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftDetection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric: Option<DriftMetric>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_interval_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_drift_exceeded: Option<DriftAction>,
}

/// Drift measurement metric (§9.2).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriftMetric {
    CosineDistance,
    EmbeddingDivergence,
}

/// Action when drift threshold is exceeded (§9.2).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriftAction {
    HaltAndReport,
    RequestReanchor,
    EscalateToHitl,
}

// --- Layer 2: Execution (§10) ---

/// Layer 2: Execution and tooling adaptation (§10).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<Sandbox>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_risk: Option<ToolRiskConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_path: Option<AllowedPath>,
}

/// Sandbox resource adaptation (§10.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sandbox {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_limits: Option<SandboxLimits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_overrides: Option<Vec<SandboxEntityOverride>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptive_limits: Option<AdaptiveLimits>,
}

/// Default sandbox resource limits.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SandboxLimits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_limit_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_limit_percent: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
}

/// Per-tool sandbox limit override.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxEntityOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_limit_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_limit_percent: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_paths: Option<Vec<String>>,
}

/// Feedback-driven limit adjustment (§10.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveLimits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram_highwater_action: Option<AdaptiveLimitAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_spike_action: Option<AdaptiveLimitAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_trending_action: Option<AdaptiveLimitAction>,
}

/// Adaptive limit trigger-action pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveLimitAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_threshold_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<LimitAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tighten_factor: Option<f64>,
}

/// Available adaptive limit actions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LimitAction {
    TightenLimit,
    LogWarning,
    QuarantineTool,
}

/// Tool risk governance (§10.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRiskConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_levels: Option<Vec<RiskLevel>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classifications: Option<Vec<ToolClassification>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obligations: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_promotion: Option<RuntimePromotion>,
}

/// Risk level definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskLevel {
    pub level: String,
    pub ordinal: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Tool-to-risk classification mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolClassification {
    pub tool_pattern: String,
    pub risk: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_hitl: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_binding: Option<String>,
}

/// Feedback-driven risk promotion (§10.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimePromotion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_violation_promotes_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consecutive_failure_threshold: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promotes_by: Option<u32>,
}

/// Allowed-path adaptation (§10.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowedPath {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_removal_on_violation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_addition_requires: Option<PathAdditionPolicy>,
}

/// Policy for adding new allowed paths.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathAdditionPolicy {
    HitlApproval,
    ConstitutionalOnly,
}

// --- Layer 3: Data Sovereignty (§11) ---

/// Layer 3: Data sovereignty adaptation (§11).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSovereignty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_cache: Option<AuthorizationCache>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reclassification: Option<Reclassification>,
}

/// Authorization caching (§11.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationCache {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_ttl_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitivity_multipliers: Option<SensitivityMultipliers>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cache_entries: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalidation_triggers: Option<Vec<InvalidationTrigger>>,
}

/// Events that invalidate cached authorization decisions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvalidationTrigger {
    ClassificationChange,
    EscalationEvent,
    CredentialRevocation,
    QuarantineEvent,
}

/// Adaptive data reclassification (§11.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reclassification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pii_frequency_escalation: Option<PiiFrequencyEscalation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_poisoning_detection: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reclassification_scope: Option<ReclassificationScope>,
}

/// PII frequency-based escalation (§11.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiiFrequencyEscalation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalation_direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_hitl_above: Option<Sensitivity>,
}

/// Scope of reclassification.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReclassificationScope {
    PerTaskClass,
    PerDataSource,
    PerAgent,
}

// --- Layer 4: Network (§12) ---

/// Layer 4: Network and mesh adaptation (§12).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Network {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub egress: Option<Egress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_routing: Option<PeerRouting>,
}

/// Adaptive egress filtering (§12.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Egress {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_deny: Option<DynamicDeny>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deny_takes_precedence: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_deny_ttl_sec: Option<u64>,
}

/// Dynamic deny entry triggers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicDeny {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_connection_failure_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_tls_error: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_suspicious_response: Option<bool>,
}

/// Peer routing and delegation (§12.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRouting {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_adaptation_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegation_roi_tracking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegation_roi_window_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefer_local_threshold: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pii_pattern_deserialization() {
        let json = r#"{
            "name": "credit_card",
            "regex": "\\d{4}[- ]?\\d{4}[- ]?\\d{4}[- ]?\\d{4}",
            "sensitivity": "restricted",
            "action": "redact"
        }"#;
        let pattern: PiiPattern = serde_json::from_str(json).unwrap();
        assert_eq!(pattern.name, "credit_card");
        assert_eq!(pattern.sensitivity, Sensitivity::Restricted);
        assert_eq!(pattern.action, PiiAction::Redact);
    }

    #[test]
    fn sandbox_limits() {
        let json = r#"{
            "default_limits": {
                "memory_limit_mb": 512,
                "cpu_limit_percent": 50,
                "timeout_sec": 30
            }
        }"#;
        let sandbox: Sandbox = serde_json::from_str(json).unwrap();
        let limits = sandbox.default_limits.unwrap();
        assert_eq!(limits.memory_limit_mb, Some(512));
        assert_eq!(limits.cpu_limit_percent, Some(50));
    }

    #[test]
    fn egress_dynamic_deny() {
        let json = r#"{
            "dynamic_deny": {
                "on_connection_failure_count": 3,
                "on_tls_error": true,
                "on_suspicious_response": true
            },
            "deny_takes_precedence": true,
            "dynamic_deny_ttl_sec": 3600
        }"#;
        let egress: Egress = serde_json::from_str(json).unwrap();
        assert_eq!(egress.deny_takes_precedence, Some(true));
        let deny = egress.dynamic_deny.unwrap();
        assert_eq!(deny.on_connection_failure_count, Some(3));
    }

    #[test]
    fn tool_risk_classification() {
        let json = r#"{
            "risk_levels": [
                {"level": "low", "ordinal": 0, "description": "Read-only"},
                {"level": "high", "ordinal": 2}
            ],
            "classifications": [
                {"tool_pattern": "git_*", "risk": "low"},
                {"tool_pattern": "shell_*", "risk": "high", "requires_hitl": true}
            ]
        }"#;
        let config: ToolRiskConfig = serde_json::from_str(json).unwrap();
        let levels = config.risk_levels.unwrap();
        assert_eq!(levels.len(), 2);
        let classifications = config.classifications.unwrap();
        assert_eq!(classifications[1].requires_hitl, Some(true));
    }
}
