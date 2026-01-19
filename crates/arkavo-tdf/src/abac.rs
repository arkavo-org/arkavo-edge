//! Attribute-Based Access Control (ABAC) evaluator for TDF policies.
//!
//! Evaluates whether a set of entitlements (from NTDF delegation) satisfies
//! the attribute requirements in a TDF policy.

use thiserror::Error;

use crate::types::Policy;

/// The result of an ABAC policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Access is permitted
    Permit,
    /// Access is denied
    Deny,
}

/// Errors that can occur during ABAC evaluation.
#[derive(Error, Debug)]
pub enum AbacError {
    /// Policy contains invalid attribute format
    #[error("Invalid attribute FQN: {0}")]
    InvalidAttributeFqn(String),

    /// Entitlement format is invalid
    #[error("Invalid entitlement format: {0}")]
    InvalidEntitlement(String),
}

/// ABAC policy evaluator.
///
/// Compares entitlements (FQN strings like `https://arkavo.net/attr/role/value/admin`)
/// against TDF policy attribute requirements.
pub struct AbacEvaluator;

impl Default for AbacEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl AbacEvaluator {
    /// Create a new ABAC evaluator.
    pub fn new() -> Self {
        Self
    }

    /// Evaluate whether the given entitlements satisfy the policy.
    ///
    /// Returns `Permit` if all policy attributes are satisfied by the entitlements,
    /// otherwise returns `Deny`.
    ///
    /// Matching rules:
    /// - Each policy attribute must have at least one matching entitlement
    /// - Entitlement matches if it grants one of the required values for the attribute
    /// - Entitlement FQN format: `{authority}/attr/{name}/value/{value}`
    /// - Policy attribute FQN format: `{authority}/attr/{name}` with required values list
    pub fn evaluate(
        &self,
        entitlements: &[String],
        policy: &Policy,
    ) -> Result<Decision, AbacError> {
        // Empty policy means no restrictions
        if policy.attributes.is_empty() {
            return Ok(Decision::Permit);
        }

        // Check each policy attribute
        for attr in &policy.attributes {
            let attr_base = &attr.attribute;

            // Find entitlements that match this attribute
            let has_matching_value = attr.values.iter().any(|required_value| {
                let expected_fqn = format!("{attr_base}/value/{required_value}");
                entitlements.contains(&expected_fqn)
            });

            if !has_matching_value {
                return Ok(Decision::Deny);
            }
        }

        Ok(Decision::Permit)
    }

    /// Parse an entitlement FQN into its components.
    ///
    /// Format: `{authority}/attr/{name}/value/{value}`
    /// Returns: `(authority, name, value)`
    pub fn parse_entitlement_fqn(fqn: &str) -> Result<(&str, &str, &str), AbacError> {
        // Find "/attr/" marker
        let attr_marker = "/attr/";
        let attr_pos = fqn
            .find(attr_marker)
            .ok_or_else(|| AbacError::InvalidEntitlement(fqn.to_string()))?;

        let authority = &fqn[..attr_pos];

        // Find "/value/" marker
        let value_marker = "/value/";
        let value_pos = fqn
            .find(value_marker)
            .ok_or_else(|| AbacError::InvalidEntitlement(fqn.to_string()))?;

        let name = &fqn[attr_pos + attr_marker.len()..value_pos];
        let value = &fqn[value_pos + value_marker.len()..];

        if authority.is_empty() || name.is_empty() || value.is_empty() {
            return Err(AbacError::InvalidEntitlement(fqn.to_string()));
        }

        Ok((authority, name, value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Attribute;

    #[test]
    fn test_permit_with_matching_entitlements() {
        let evaluator = AbacEvaluator::new();

        let policy = Policy {
            id: None,
            attributes: vec![Attribute::new(
                "https://arkavo.net/attr/role",
                &["admin", "operator"],
            )],
            dissemination: vec![],
        };

        let entitlements = vec![
            "https://arkavo.net/attr/role/value/admin".to_string(),
            "https://arkavo.net/attr/clearance/value/secret".to_string(),
        ];

        let result = evaluator.evaluate(&entitlements, &policy).unwrap();
        assert_eq!(result, Decision::Permit);
    }

    #[test]
    fn test_deny_with_missing_entitlement() {
        let evaluator = AbacEvaluator::new();

        let policy = Policy {
            id: None,
            attributes: vec![Attribute::new(
                "https://arkavo.net/attr/role",
                &["admin"],
            )],
            dissemination: vec![],
        };

        let entitlements = vec![
            "https://arkavo.net/attr/role/value/user".to_string(), // Has user, not admin
        ];

        let result = evaluator.evaluate(&entitlements, &policy).unwrap();
        assert_eq!(result, Decision::Deny);
    }

    #[test]
    fn test_permit_with_empty_policy() {
        let evaluator = AbacEvaluator::new();

        let policy = Policy::default();

        let entitlements = vec![
            "https://arkavo.net/attr/role/value/user".to_string(),
        ];

        let result = evaluator.evaluate(&entitlements, &policy).unwrap();
        assert_eq!(result, Decision::Permit);
    }

    #[test]
    fn test_deny_with_empty_entitlements() {
        let evaluator = AbacEvaluator::new();

        let policy = Policy {
            id: None,
            attributes: vec![Attribute::new(
                "https://arkavo.net/attr/role",
                &["admin"],
            )],
            dissemination: vec![],
        };

        let entitlements: Vec<String> = vec![];

        let result = evaluator.evaluate(&entitlements, &policy).unwrap();
        assert_eq!(result, Decision::Deny);
    }

    #[test]
    fn test_multiple_attributes_all_required() {
        let evaluator = AbacEvaluator::new();

        let policy = Policy {
            id: None,
            attributes: vec![
                Attribute::new("https://arkavo.net/attr/role", &["admin"]),
                Attribute::new("https://arkavo.net/attr/clearance", &["secret", "top-secret"]),
            ],
            dissemination: vec![],
        };

        // Has admin but only "confidential" clearance (not secret or top-secret)
        let entitlements = vec![
            "https://arkavo.net/attr/role/value/admin".to_string(),
            "https://arkavo.net/attr/clearance/value/confidential".to_string(),
        ];

        let result = evaluator.evaluate(&entitlements, &policy).unwrap();
        assert_eq!(result, Decision::Deny);

        // Has both admin role and secret clearance
        let entitlements = vec![
            "https://arkavo.net/attr/role/value/admin".to_string(),
            "https://arkavo.net/attr/clearance/value/secret".to_string(),
        ];

        let result = evaluator.evaluate(&entitlements, &policy).unwrap();
        assert_eq!(result, Decision::Permit);
    }

    #[test]
    fn test_parse_entitlement_fqn() {
        let fqn = "https://arkavo.net/attr/role/value/admin";
        let (authority, name, value) = AbacEvaluator::parse_entitlement_fqn(fqn).unwrap();

        assert_eq!(authority, "https://arkavo.net");
        assert_eq!(name, "role");
        assert_eq!(value, "admin");
    }

    #[test]
    fn test_parse_entitlement_fqn_invalid() {
        let invalid = "https://arkavo.net/role/admin";
        let result = AbacEvaluator::parse_entitlement_fqn(invalid);
        assert!(matches!(result, Err(AbacError::InvalidEntitlement(_))));
    }

    #[test]
    fn test_alternative_values_any_match() {
        let evaluator = AbacEvaluator::new();

        let policy = Policy {
            id: None,
            attributes: vec![Attribute::new(
                "https://arkavo.net/attr/department",
                &["engineering", "security", "devops"],
            )],
            dissemination: vec![],
        };

        // Has "security" which is one of the allowed values
        let entitlements = vec![
            "https://arkavo.net/attr/department/value/security".to_string(),
        ];

        let result = evaluator.evaluate(&entitlements, &policy).unwrap();
        assert_eq!(result, Decision::Permit);
    }
}
