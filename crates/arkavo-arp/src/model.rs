//! Core ARP document types: top-level document, ADL reference, integrity, and shared primitives.

use serde::{Deserialize, Serialize};

use crate::adaptation::Adaptation;
use crate::constraints::{Budget, Escalation, Hitl, Quarantine, Session};
use crate::feedback::FeedbackLoops;
use crate::layers::{Cognitive, DataSovereignty, Execution, Network};
use crate::observability::{Observability, StateStorage};
use crate::proposal::ProposalPolicy;

/// Top-level ARP document per §4.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArpDocument {
    pub arp_spec: String,
    pub adl_ref: AdlRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity: Option<Integrity>,
    pub adaptation: Adaptation,
    pub feedback_loops: FeedbackLoops,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precedence: Option<Precedence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cognitive: Option<Cognitive>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution: Option<Execution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_sovereignty: Option<DataSovereignty>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Network>,
    pub budget: Budget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalation: Option<Escalation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quarantine: Option<Quarantine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hitl: Option<Hitl>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<Session>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_storage: Option<StateStorage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observability: Option<Observability>,
    /// Governs which tightening proposals this agent ingests (§18.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal_policy: Option<ProposalPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Cryptographic binding to the companion ADL document (§4.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdlRef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_hash: Option<String>,
}

/// Cryptographic integrity and trust binding (§5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Integrity {
    pub signed_by: String,
    pub algorithm: String,
    pub signature: String,
    pub signed_content: SignedContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest_algorithm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// Whether the signature covers the canonical form or a digest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignedContent {
    Canonical,
    Digest,
}

/// Beta distribution parameters used throughout ARP for Bayesian priors.
///
/// Both `alpha` and `beta` must be positive. Values ≤ 0 are clamped to a
/// minimum of `f64::EPSILON` during deserialization and in `new()`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct BetaPrior {
    pub alpha: f64,
    pub beta: f64,
}

impl BetaPrior {
    /// Create a new `BetaPrior`, enforcing positive values.
    pub fn new(alpha: f64, beta: f64) -> Self {
        Self {
            alpha: alpha.max(f64::EPSILON),
            beta: beta.max(f64::EPSILON),
        }
    }

    /// Clamp both parameters to their minimum valid values.
    pub fn sanitize(self) -> Self {
        Self::new(self.alpha, self.beta)
    }
}

impl Default for BetaPrior {
    fn default() -> Self {
        Self {
            alpha: 2.0,
            beta: 1.0,
        }
    }
}

/// Data sensitivity levels used across multiple ARP sections.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Public,
    Internal,
    Confidential,
    Restricted,
}

/// Order of precedence and conflict resolution (§8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Precedence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
}

/// Per-sensitivity-level TTL multipliers used in auth cache and session timeout.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SensitivityMultipliers {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub internal: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidential: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restricted: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beta_prior_default() {
        let prior = BetaPrior::default();
        assert_eq!(prior.alpha, 2.0);
        assert_eq!(prior.beta, 1.0);
    }

    #[test]
    fn sensitivity_ordering() {
        assert!(Sensitivity::Public < Sensitivity::Internal);
        assert!(Sensitivity::Internal < Sensitivity::Confidential);
        assert!(Sensitivity::Confidential < Sensitivity::Restricted);
    }

    #[test]
    fn adl_ref_with_uri_only() {
        let json = r#"{"uri": "https://example.com/adl.json"}"#;
        let adl_ref: AdlRef = serde_json::from_str(json).unwrap();
        assert!(adl_ref.uri.is_some());
        assert!(adl_ref.document_hash.is_none());
    }

    #[test]
    fn adl_ref_with_hash_only() {
        let json = r#"{"document_hash": "sha256:abc123"}"#;
        let adl_ref: AdlRef = serde_json::from_str(json).unwrap();
        assert!(adl_ref.uri.is_none());
        assert_eq!(adl_ref.document_hash.as_deref(), Some("sha256:abc123"));
    }

    #[test]
    fn signed_content_serialization() {
        let canonical = SignedContent::Canonical;
        let json = serde_json::to_string(&canonical).unwrap();
        assert_eq!(json, r#""canonical""#);

        let digest = SignedContent::Digest;
        let json = serde_json::to_string(&digest).unwrap();
        assert_eq!(json, r#""digest""#);
    }
}
