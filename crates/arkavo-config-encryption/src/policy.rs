use serde::{Deserialize, Serialize};

/// Arkavo attribute authority for TDF policies
pub const ARKAVO_AUTHORITY: &str = "https://arkavo.ai";

/// Create an Arkavo attribute FQN following OpenTDF spec
/// Example: create_fqn("data/clearance", "confidential")
///       -> "https://arkavo.ai/attr/data/clearance/value/confidential"
pub fn create_fqn(attribute: &str, value: &str) -> String {
    format!("{}/attr/{}/value/{}", ARKAVO_AUTHORITY, attribute, value)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub attributes: Vec<PolicyAttribute>,
    pub dissemination: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyAttribute {
    pub attribute: String,
    pub value: String,
    pub display_name: String,
}

impl Policy {
    pub fn new() -> Self {
        Self {
            attributes: Vec::new(),
            dissemination: Vec::new(),
        }
    }

    pub fn add_attribute(&mut self, attribute: String, value: String, display_name: String) {
        self.attributes.push(PolicyAttribute {
            attribute,
            value,
            display_name,
        });
    }

    pub fn add_dissemination(&mut self, rule: String) {
        self.dissemination.push(rule);
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_policy() {
        let mut policy = Policy::new();
        policy.add_attribute(
            "data/clearance".to_string(),
            "internal".to_string(),
            "Data Clearance".to_string(),
        );
        policy.add_dissemination("agent.role=test-agent".to_string());

        assert_eq!(policy.attributes.len(), 1);
        assert_eq!(policy.dissemination.len(), 1);
        assert_eq!(policy.attributes[0].attribute, "data/clearance");
        assert_eq!(policy.attributes[0].value, "internal");
    }

    #[test]
    fn test_create_fqn() {
        let fqn = create_fqn("data/clearance", "confidential");
        assert_eq!(
            fqn,
            "https://arkavo.ai/attr/data/clearance/value/confidential"
        );

        let fqn2 = create_fqn("capability/filesystem", "read_content");
        assert_eq!(
            fqn2,
            "https://arkavo.ai/attr/capability/filesystem/value/read_content"
        );
    }
}
