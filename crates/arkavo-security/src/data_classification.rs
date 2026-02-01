//! Data Classification System

/// Data category
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataCategory {
    /// Personally Identifiable Information
    Pii,
    /// Credentials (passwords, API keys)
    Credentials,
    /// Financial data
    Financial,
    /// Healthcare data
    Healthcare,
    /// Internal data
    Internal,
    /// Public data
    Public,
}

/// Sensitivity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SensitivityLevel {
    /// Public data
    Public = 0,
    /// Internal use only
    Internal = 1,
    /// Confidential
    Confidential = 2,
    /// Restricted access
    Restricted = 3,
}

/// Individual datum types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DatumType {
    /// Email address
    Email,
    /// Phone number
    PhoneNumber,
    /// Social security number
    SocialSecurityNumber,
    /// API key
    ApiKey,
    /// Password
    Password,
    /// Credit card number
    CreditCardNumber,
    /// Internal IP address
    InternalIpAddress,
}

impl DatumType {
    /// Get the category for this datum type
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

    /// Get the default sensitivity for this datum type
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
#[derive(Debug, Clone)]
pub struct ClassifiedDatum {
    /// The datum type
    pub datum_type: DatumType,
    /// Position in the text
    pub position: (usize, usize),
    /// The matched text
    pub matched_text: String,
}

impl ClassifiedDatum {
    /// Get the sensitivity level
    pub fn sensitivity(&self) -> SensitivityLevel {
        self.datum_type.default_sensitivity()
    }

    /// Get the category
    pub fn category(&self) -> DataCategory {
        self.datum_type.category()
    }
}

/// DLP Action
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DlpAction {
    /// Allow the data
    Allow,
    /// Block the data
    Block,
    /// Redact the data
    Redact,
}

/// DLP Policy
pub struct DlpPolicy {
    /// Sensitivity threshold
    pub threshold: SensitivityLevel,
    /// Action to take
    pub action: DlpAction,
}

impl DlpPolicy {
    /// Create a strict policy
    pub fn strict() -> Self {
        Self {
            threshold: SensitivityLevel::Restricted,
            action: DlpAction::Block,
        }
    }

    /// Evaluate a classified datum against this policy
    pub fn evaluate(&self, classification: &ClassifiedDatum) -> DlpAction {
        if classification.sensitivity() >= self.threshold {
            self.action
        } else {
            DlpAction::Allow
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_datum_type_category() {
        assert_eq!(DatumType::Email.category(), DataCategory::Pii);
        assert_eq!(DatumType::Password.category(), DataCategory::Credentials);
        assert_eq!(
            DatumType::CreditCardNumber.category(),
            DataCategory::Financial
        );
    }

    #[test]
    fn test_datum_type_sensitivity() {
        assert_eq!(
            DatumType::Password.default_sensitivity(),
            SensitivityLevel::Restricted
        );
        assert_eq!(
            DatumType::ApiKey.default_sensitivity(),
            SensitivityLevel::Restricted
        );
        assert_eq!(
            DatumType::CreditCardNumber.default_sensitivity(),
            SensitivityLevel::Confidential
        );
        assert_eq!(
            DatumType::Email.default_sensitivity(),
            SensitivityLevel::Internal
        );
    }

    #[test]
    fn test_dlp_policy() {
        let policy = DlpPolicy::strict();

        let classified = ClassifiedDatum {
            datum_type: DatumType::Password,
            position: (0, 10),
            matched_text: "password123".to_string(),
        };

        assert_eq!(policy.evaluate(&classified), DlpAction::Block);

        let classified_low = ClassifiedDatum {
            datum_type: DatumType::Email,
            position: (0, 10),
            matched_text: "test@example.com".to_string(),
        };

        // Email is Internal level, which is less than Restricted threshold
        assert_eq!(policy.evaluate(&classified_low), DlpAction::Allow);
    }

    #[test]
    fn test_classified_datum_helpers() {
        let classified = ClassifiedDatum {
            datum_type: DatumType::SocialSecurityNumber,
            position: (0, 11),
            matched_text: "123-45-6789".to_string(),
        };

        assert_eq!(classified.sensitivity(), SensitivityLevel::Restricted);
        assert_eq!(classified.category(), DataCategory::Pii);
    }
}
