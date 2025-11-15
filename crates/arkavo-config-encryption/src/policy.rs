use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub attributes: Vec<PolicyAttribute>,
    pub dissemination: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyAttribute {
    pub attribute: String,
    pub display_name: String,
}

impl Policy {
    pub fn new() -> Self {
        Self {
            attributes: Vec::new(),
            dissemination: Vec::new(),
        }
    }

    pub fn add_attribute(&mut self, attribute: String, display_name: String) {
        self.attributes.push(PolicyAttribute {
            attribute,
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
        policy.add_attribute("agent.role".to_string(), "Agent Role".to_string());
        policy.add_dissemination("agent.role=test-agent".to_string());

        assert_eq!(policy.attributes.len(), 1);
        assert_eq!(policy.dissemination.len(), 1);
        assert_eq!(policy.attributes[0].attribute, "agent.role");
    }
}