//! Policy builder for constructing TDF access policies.

use crate::error::TdfError;
use crate::types::{Attribute, Policy};
use uuid::Uuid;

/// Builder for constructing TDF policies with attribute-based access control.
///
/// # Example
///
/// ```
/// use arkavo_tdf::PolicyBuilder;
///
/// let policy = PolicyBuilder::new()
///     .attribute("https://arkavo.net/attr/role", &["admin"])
///     .attribute("https://arkavo.net/attr/clearance", &["secret"])
///     .dissemination(&["US", "CA"])
///     .build()
///     .unwrap();
///
/// assert_eq!(policy.attributes.len(), 2);
/// assert_eq!(policy.dissemination, vec!["US", "CA"]);
/// ```
#[derive(Debug, Clone, Default)]
pub struct PolicyBuilder {
    id: Option<String>,
    attributes: Vec<Attribute>,
    dissemination: Vec<String>,
}

impl PolicyBuilder {
    /// Create a new policy builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the policy ID (auto-generated UUID if not set).
    #[must_use]
    pub fn id(mut self, id: &str) -> Self {
        self.id = Some(id.to_string());
        self
    }

    /// Add an attribute requirement to the policy.
    ///
    /// The attribute FQN should be a fully qualified name like:
    /// `https://arkavo.net/attr/role`
    #[must_use]
    pub fn attribute(mut self, fqn: &str, values: &[&str]) -> Self {
        self.attributes.push(Attribute::new(fqn, values));
        self
    }

    /// Add an attribute with a single value.
    #[must_use]
    pub fn attribute_single(self, fqn: &str, value: &str) -> Self {
        self.attribute(fqn, &[value])
    }

    /// Set dissemination controls (e.g., country codes).
    #[must_use]
    pub fn dissemination(mut self, controls: &[&str]) -> Self {
        self.dissemination = controls.iter().map(|s| (*s).to_string()).collect();
        self
    }

    /// Add a single dissemination control.
    #[must_use]
    pub fn add_dissemination(mut self, control: &str) -> Self {
        self.dissemination.push(control.to_string());
        self
    }

    /// Build the policy, validating all fields.
    ///
    /// # Errors
    ///
    /// Returns `TdfError::Policy` if:
    /// - No attributes are specified (empty policy)
    /// - An attribute has no values
    /// - An attribute FQN is invalid
    pub fn build(self) -> Result<Policy, TdfError> {
        if self.attributes.is_empty() {
            return Err(TdfError::Policy(
                "Policy must have at least one attribute".to_string(),
            ));
        }

        for attr in &self.attributes {
            if attr.values.is_empty() {
                return Err(TdfError::Policy(format!(
                    "Attribute '{}' must have at least one value",
                    attr.attribute
                )));
            }

            if !attr.attribute.starts_with("https://") && !attr.attribute.starts_with("http://") {
                return Err(TdfError::Policy(format!(
                    "Attribute FQN must be a URL: '{}'",
                    attr.attribute
                )));
            }
        }

        let id = self.id.unwrap_or_else(|| Uuid::new_v4().to_string());

        Ok(Policy {
            id: Some(id),
            attributes: self.attributes,
            dissemination: self.dissemination,
        })
    }

    /// Build the policy without validation (for testing).
    #[must_use]
    pub fn build_unchecked(self) -> Policy {
        Policy {
            id: self.id,
            attributes: self.attributes,
            dissemination: self.dissemination,
        }
    }
}

/// Common attribute namespace for Arkavo.
pub mod arkavo_attrs {
    /// Base namespace for Arkavo attributes.
    pub const NAMESPACE: &str = "https://arkavo.net/attr";

    /// Role-based access control attribute.
    pub const ROLE: &str = "https://arkavo.net/attr/role";

    /// Security clearance level attribute.
    pub const CLEARANCE: &str = "https://arkavo.net/attr/clearance";

    /// MCP tool access attribute.
    pub const MCP_TOOL: &str = "https://arkavo.net/attr/mcp-tool";

    /// Agent identity attribute.
    pub const AGENT_ID: &str = "https://arkavo.net/attr/agent-id";

    /// Organization membership attribute.
    pub const ORGANIZATION: &str = "https://arkavo.net/attr/organization";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_basic() {
        let policy = PolicyBuilder::new()
            .attribute(arkavo_attrs::ROLE, &["admin"])
            .build()
            .unwrap();

        assert!(policy.id.is_some());
        assert_eq!(policy.attributes.len(), 1);
        assert_eq!(policy.attributes[0].attribute, arkavo_attrs::ROLE);
        assert_eq!(policy.attributes[0].values, vec!["admin"]);
    }

    #[test]
    fn builder_multiple_attributes() {
        let policy = PolicyBuilder::new()
            .attribute(arkavo_attrs::ROLE, &["admin", "user"])
            .attribute(arkavo_attrs::CLEARANCE, &["secret"])
            .dissemination(&["US", "CA", "UK"])
            .build()
            .unwrap();

        assert_eq!(policy.attributes.len(), 2);
        assert_eq!(policy.dissemination.len(), 3);
    }

    #[test]
    fn builder_custom_id() {
        let policy = PolicyBuilder::new()
            .id("custom-policy-id")
            .attribute(arkavo_attrs::ROLE, &["user"])
            .build()
            .unwrap();

        assert_eq!(policy.id, Some("custom-policy-id".to_string()));
    }

    #[test]
    fn builder_attribute_single() {
        let policy = PolicyBuilder::new()
            .attribute_single(arkavo_attrs::AGENT_ID, "agent-123")
            .build()
            .unwrap();

        assert_eq!(policy.attributes[0].values, vec!["agent-123"]);
    }

    #[test]
    fn builder_add_dissemination() {
        let policy = PolicyBuilder::new()
            .attribute(arkavo_attrs::ROLE, &["user"])
            .add_dissemination("US")
            .add_dissemination("CA")
            .build()
            .unwrap();

        assert_eq!(policy.dissemination, vec!["US", "CA"]);
    }

    #[test]
    fn builder_error_empty_attributes() {
        let result = PolicyBuilder::new().build();
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("at least one attribute"));
    }

    #[test]
    fn builder_error_empty_values() {
        let result = PolicyBuilder::new()
            .attribute(arkavo_attrs::ROLE, &[])
            .build();

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("at least one value"));
    }

    #[test]
    fn builder_error_invalid_fqn() {
        let result = PolicyBuilder::new()
            .attribute("invalid-fqn", &["value"])
            .build();

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("must be a URL"));
    }

    #[test]
    fn builder_unchecked() {
        let policy = PolicyBuilder::new()
            .attribute("not-a-url", &[])
            .build_unchecked();

        assert_eq!(policy.attributes[0].attribute, "not-a-url");
        assert!(policy.attributes[0].values.is_empty());
    }

    #[test]
    fn arkavo_attrs_constants() {
        assert!(arkavo_attrs::ROLE.starts_with(arkavo_attrs::NAMESPACE));
        assert!(arkavo_attrs::CLEARANCE.starts_with(arkavo_attrs::NAMESPACE));
        assert!(arkavo_attrs::MCP_TOOL.starts_with(arkavo_attrs::NAMESPACE));
    }
}
