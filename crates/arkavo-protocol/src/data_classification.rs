//! Data Classification System

use serde::{Deserialize, Serialize};

/// Data category
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataCategory {
    Pii,
    Credentials,
    Financial,
    Healthcare,
    Internal,
    Public,
}

/// Sensitivity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensitivityLevel {
    Public = 0,
    Internal = 1,
    Confidential = 2,
    Restricted = 3,
}

/// Individual datum types
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatumType {
    Email,
    PhoneNumber,
    SocialSecurityNumber,
    ApiKey,
    Password,
    CreditCardNumber,
    InternalIpAddress,
}

impl DatumType {
    pub fn category(&self) -> DataCategory {
        match self {
            DatumType::Email | DatumType::PhoneNumber | DatumType::SocialSecurityNumber => {
                DataCategory::Pii
            }
            DatumType::ApiKey | DatumType::Password => DataCategory::Credentials,
            DatumType::CreditCardNumber => DataCategory::Financial,
            DatumType::InternalIpAddress => DataCategory::Internal,
        }
    }

    pub fn default_sensitivity(&self) -> SensitivityLevel {
        match self {
            DatumType::Password | DatumType::ApiKey | DatumType::SocialSecurityNumber => {
                SensitivityLevel::Restricted
            }
            DatumType::CreditCardNumber => SensitivityLevel::Confidential,
            DatumType::Email | DatumType::PhoneNumber | DatumType::InternalIpAddress => {
                SensitivityLevel::Internal
            }
        }
    }
}

/// Classification result
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifiedDatum {
    pub datum_type: DatumType,
    pub position: (usize, usize),
    pub matched_text: String,
}

impl ClassifiedDatum {
    pub fn sensitivity(&self) -> SensitivityLevel {
        self.datum_type.default_sensitivity()
    }

    pub fn category(&self) -> DataCategory {
        self.datum_type.category()
    }
}

/// DLP Action
///
/// `Wrap` and `Hold` exist because Allow/Block/Redact cannot express two things
/// the egress gate has to say. A payload the requester is entitled to but the
/// transport is not may travel wrapped rather than not at all; and a decision
/// the gate could not reach — an unresolved destination, an exhausted budget, a
/// detector that did not answer — must hold the content rather than resolve to
/// one of the two terminal answers (SENT-013).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DlpAction {
    Allow,
    Block,
    Redact,
    /// Deliver only under these OpenTDF attributes, as (fqn, value) pairs.
    Wrap {
        attributes: Vec<(String, String)>,
    },
    /// Neither released nor refused: held pending resolution.
    Hold,
}

/// DLP Policy
pub struct DlpPolicy {
    pub threshold: SensitivityLevel,
    pub action: DlpAction,
}

impl DlpPolicy {
    pub fn strict() -> Self {
        Self {
            threshold: SensitivityLevel::Restricted,
            action: DlpAction::Block,
        }
    }

    pub fn evaluate(&self, classification: &ClassifiedDatum) -> DlpAction {
        if classification.sensitivity() >= self.threshold {
            self.action.clone()
        } else {
            DlpAction::Allow
        }
    }
}
