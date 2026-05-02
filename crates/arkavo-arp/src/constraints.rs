//! Budget, escalation, quarantine, HITL, and session types (§13-§15).

use serde::{Deserialize, Serialize};

use crate::model::SensitivityMultipliers;

// --- Budget (§13) ---

/// Budget and throughput constraints (§13).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub task_ceiling_usd: f64,
    pub on_exhaustion: BudgetExhaustionAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degradation_chain: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert_threshold_pct: Option<f64>,
    pub velocity: Velocity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_layer: Option<PerLayerBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limiting: Option<RateLimiting>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accounting: Option<Accounting>,
}

/// Action when budget is exhausted (§13.1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetExhaustionAction {
    HaltAndReport,
    DegradeToCheapestModel,
    QueueForHumanApproval,
}

/// Velocity constraints (§13.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Velocity {
    pub max_spend_per_minute_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_calls_per_minute: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens_per_minute: Option<u64>,
}

/// Per-layer budget limits (§13.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerLayerBudget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cognitive: Option<LayerBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution: Option<LayerBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<LayerBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<LayerBudget>,
}

/// Budget limit for a single layer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LayerBudget {
    #[serde(alias = "per_completion_limit_usd")]
    #[serde(alias = "per_tool_call_limit_usd")]
    #[serde(alias = "per_retrieval_limit_usd")]
    #[serde(alias = "per_delegation_limit_usd")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_usd: Option<f64>,
}

/// Rate limiting (§13.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimiting {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global: Option<RateLimit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_endpoint: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_tool: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tracking_entries: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracking_entry_ttl_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_interval_sec: Option<u64>,
}

/// Rate limit parameters.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RateLimit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requests_per_second: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub burst_size: Option<u64>,
}

/// Budget accounting (§13.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Accounting {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution: Option<Attribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollup_window_sec: Option<u64>,
}

/// Budget attribution scope.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Attribution {
    PerAgent,
    PerTask,
}

// --- Escalation (§14) ---

/// Cross-layer escalation (§14.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Escalation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cross_layer_rules: Option<Vec<EscalationRule>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratchet_direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relaxation_requires: Option<String>,
}

/// Escalation trigger-action pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationRule {
    pub trigger: EscalationTrigger,
    pub actions: Vec<EscalationAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_threshold: Option<f64>,
}

/// Event that triggers an escalation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationTrigger {
    pub layer: String,
    pub event: String,
}

/// Action taken in response to an escalation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationAction {
    pub target_layer: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

// --- Quarantine (§14.2) ---

/// Quarantine configuration (§14.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quarantine {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_ttl_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_threshold_for_sticky: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_formula: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sticky_requires: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<QuarantineScope>>,
}

/// Entities that can be quarantined.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuarantineScope {
    Tool,
    Model,
    Peer,
    DataSource,
    Endpoint,
}

// --- HITL (§15) ---

/// Human-In-The-Loop configuration (§15).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hitl {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relaxation: Option<HitlRelaxation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teaching: Option<HitlTeaching>,
}

/// Ratchet relaxation rules (§15.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlRelaxation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relaxation_conditions: Option<Vec<RelaxationCondition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_mechanism: Option<ApprovalMechanism>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_requirement: Option<String>,
}

/// Condition for permitting ratchet relaxation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelaxationCondition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<RelaxationScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_credential: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_role: Option<String>,
}

/// Scopes that can be relaxed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RelaxationScope {
    Quarantine,
    Classification,
    BudgetCeiling,
    RiskLevel,
    DriftThreshold,
}

/// Approval mechanism for relaxation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMechanism {
    CredentialPresentation,
    SignedCommand,
    InteractiveConfirmation,
}

/// Teaching interface configuration (§15.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlTeaching {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_classification: Option<Vec<TeachingIntent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_cache_write: Option<serde_json::Value>,
}

/// Teaching intent classification.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TeachingIntent {
    Instruction,
    Correction,
    Reinforcement,
    Pattern,
}

// --- Session (§15.3) ---

/// Session management (§15.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_bounds: Option<TimeoutBounds>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defaults: Option<SessionDefaults>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitivity_proportional: Option<SensitivityProportional>,
}

/// Bounds on configurable timeouts.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TimeoutBounds {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absolute_timeout_min_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absolute_timeout_max_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_timeout_max_sec: Option<u64>,
}

/// Default session timeout values.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SessionDefaults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absolute_timeout_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_timeout_sec: Option<u64>,
}

/// Sensitivity-proportional timeout scaling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensitivityProportional {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_timeout_multipliers: Option<SensitivityMultipliers>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_deserialization() {
        let json = r#"{
            "task_ceiling_usd": 2.50,
            "on_exhaustion": "halt_and_report",
            "velocity": {"max_spend_per_minute_usd": 0.50},
            "rate_limiting": {
                "global": {"requests_per_second": 100.0, "burst_size": 10}
            }
        }"#;
        let budget: Budget = serde_json::from_str(json).unwrap();
        assert_eq!(budget.task_ceiling_usd, 2.50);
        assert_eq!(budget.on_exhaustion, BudgetExhaustionAction::HaltAndReport);
        let rl = budget.rate_limiting.unwrap();
        let global = rl.global.unwrap();
        assert_eq!(global.requests_per_second, Some(100.0));
    }

    #[test]
    fn escalation_rules() {
        let json = r#"{
            "cross_layer_rules": [
                {
                    "trigger": {"layer": "execution", "event": "sandbox_violation"},
                    "actions": [
                        {"target_layer": "cognitive", "action": "tighten_drift"},
                        {"target_layer": "network", "action": "deny_egress"}
                    ],
                    "confidence_threshold": 0.8
                }
            ],
            "ratchet_direction": "tighten_only",
            "relaxation_requires": "human_approval"
        }"#;
        let esc: Escalation = serde_json::from_str(json).unwrap();
        let rules = esc.cross_layer_rules.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].trigger.event, "sandbox_violation");
        assert_eq!(rules[0].actions.len(), 2);
    }

    #[test]
    fn session_timeout_bounds() {
        let json = r#"{
            "timeout_bounds": {
                "absolute_timeout_min_sec": 60,
                "absolute_timeout_max_sec": 86400,
                "idle_timeout_max_sec": 3600
            },
            "defaults": {
                "absolute_timeout_sec": 3600,
                "idle_timeout_sec": 900
            }
        }"#;
        let session: Session = serde_json::from_str(json).unwrap();
        let bounds = session.timeout_bounds.unwrap();
        assert_eq!(bounds.absolute_timeout_min_sec, Some(60));
        assert_eq!(bounds.absolute_timeout_max_sec, Some(86400));
        let defaults = session.defaults.unwrap();
        assert_eq!(defaults.absolute_timeout_sec, Some(3600));
    }

    #[test]
    fn quarantine_scopes() {
        let json = r#"{
            "default_ttl_sec": 3600,
            "confidence_threshold_for_sticky": 0.7,
            "sticky_requires": "human_review",
            "scopes": ["tool", "model", "peer"]
        }"#;
        let q: Quarantine = serde_json::from_str(json).unwrap();
        let scopes = q.scopes.unwrap();
        assert_eq!(scopes.len(), 3);
        assert_eq!(scopes[0], QuarantineScope::Tool);
    }
}
