//! The messages around a delegation envelope (spec §7.3.2 to §7.3.4, §7.4).

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// JSON-RPC method names (§7.2, §7.3.4, §7.4).
pub mod methods {
    pub const DELEGATE: &str = "swarmkit/delegate";
    pub const STATUS: &str = "swarmkit/status";
    pub const REVOKE: &str = "swarmkit/revoke";
}

/// JSON-RPC error code of a refused delegation or revocation (§7.3.3).
pub const REFUSAL_CODE: i32 = -32040;

/// Why a specialist refused (§7.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    InvalidEnvelope,
    BadSignature,
    WrongSpecialist,
    NotYetValid,
    Expired,
    Replayed,
    NotBoundOrchestrator,
    BindingRequired,
    HostUnsuitable,
    SkillUnverified,
}

/// The `data` of a refusal: its reason and what triggered it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refusal {
    pub reason: RefusalReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Refusal {
    pub fn new(reason: RefusalReason, detail: impl Into<String>) -> Self {
        Self {
            reason,
            detail: Some(detail.into()),
        }
    }

    /// The JSON-RPC error message: the reason in words.
    pub fn message(&self) -> &'static str {
        match self.reason {
            RefusalReason::InvalidEnvelope => "invalid envelope",
            RefusalReason::BadSignature => "signature does not verify",
            RefusalReason::WrongSpecialist => "addressed to another specialist",
            RefusalReason::NotYetValid => "not yet valid",
            RefusalReason::Expired => "expired",
            RefusalReason::Replayed => "already received",
            RefusalReason::NotBoundOrchestrator => "bound to another orchestrator",
            RefusalReason::BindingRequired => "not bound and does not bind on first use",
            RefusalReason::HostUnsuitable => "this host cannot take the role",
            RefusalReason::SkillUnverified => "a skill signature does not verify",
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{}: {detail}", self.message()),
            None => f.write_str(self.message()),
        }
    }
}

impl std::error::Error for Refusal {}

/// Result of an applied delegation (§7.3.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ready {
    pub status: ReadyStatus,
    pub specialist: String,
    pub flight_id: Uuid,
    pub role_id: String,
    pub envelope_blake3: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadyStatus {
    Ready,
}

/// Result of an accepted revocation (§7.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revoked {
    pub status: RevokedStatus,
    pub specialist: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevokedStatus {
    Revoked,
}

/// How a binding was made (§7.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BindingMode {
    Operator,
    Pairing,
    FirstUse,
}

/// The orchestrator a specialist accepts delegations from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub orchestrator: String,
    pub mode: BindingMode,
    pub since: DateTime<Utc>,
}

/// The active delegation, as status reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationSummary {
    pub flight_id: Uuid,
    pub kit_id: String,
    pub role_id: String,
    pub expires: DateTime<Utc>,
}

/// A capability the host can provide, whatever role it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityReport {
    pub name: String,
    pub ready: bool,
}

/// Result of `swarmkit/status` (§7.3.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecialistStatus {
    pub specialist: String,
    pub binding: Option<Binding>,
    pub binds_by: Vec<BindingMode>,
    pub delegation: Option<DelegationSummary>,
    pub capabilities: Vec<CapabilityReport>,
    pub load: f64,
}

impl SpecialistStatus {
    /// Whether the host reports `name`, and whether it is ready.
    pub fn capability(&self, name: &str) -> Option<bool> {
        self.capabilities
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_follow_the_spec() {
        let refusal = Refusal::new(RefusalReason::NotBoundOrchestrator, "bound to did:key:z6Mk");
        assert_eq!(
            serde_json::to_value(&refusal).unwrap(),
            serde_json::json!({"reason": "not_bound_orchestrator", "detail": "bound to did:key:z6Mk"})
        );
        assert_eq!(
            serde_json::to_value(BindingMode::FirstUse).unwrap(),
            serde_json::json!("first-use")
        );
        assert_eq!(
            refusal.to_string(),
            "bound to another orchestrator: bound to did:key:z6Mk"
        );
    }
}
