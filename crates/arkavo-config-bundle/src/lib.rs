use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

pub mod error;
pub use error::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigurationBundle {
    pub bundle_id: Uuid,
    pub target: BundleTarget,
    pub settings: HashMap<String, serde_json::Value>,
    pub role: AgentRole,
    pub entitlements: Vec<Entitlement>,
    pub secrets: HashMap<String, Secret>,
    pub required_attributes: Vec<String>,
    pub metadata: BundleMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value")]
pub enum BundleTarget {
    Agent(String),
    Role(String),
    AttributePattern(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentRole {
    pub name: String,
    pub purpose: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entitlement {
    pub resource_type: ResourceType,
    pub resource_id: String,
    pub permissions: Vec<Permission>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    McpTool,
    DataProvider,
    FileSystem,
    Network,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Execute,
    Read,
    Write,
    Delete,
    ReadResults,
    Connect,
    Query,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Secret {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub secret_type: SecretType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_policy: Option<RotationPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SecretMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SecretType {
    ApiKey,
    Token,
    Certificate,
    Password,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RotationPolicy {
    pub interval_days: u32,
    pub auto_rotate: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_before_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecretMetadata {
    pub created_at: DateTime<Utc>,
    pub last_rotated: DateTime<Utc>,
    pub next_rotation: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BundleMetadata {
    pub version: String,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,
    pub environment: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl ConfigurationBundle {
    pub fn new(
        target: BundleTarget,
        role: AgentRole,
        created_by: String,
        environment: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            bundle_id: Uuid::new_v4(),
            target,
            settings: HashMap::new(),
            role,
            entitlements: Vec::new(),
            secrets: HashMap::new(),
            required_attributes: Vec::new(),
            metadata: BundleMetadata {
                version: "1.0.0".to_string(),
                created_at: now,
                created_by: created_by.clone(),
                updated_at: now,
                updated_by: created_by,
                environment,
                tags: None,
                description: None,
            },
        }
    }

    pub fn add_setting(&mut self, key: String, value: serde_json::Value) {
        self.settings.insert(key, value);
    }

    pub fn add_entitlement(&mut self, entitlement: Entitlement) {
        self.entitlements.push(entitlement);
    }

    pub fn add_secret(&mut self, name: String, secret: Secret) {
        self.secrets.insert(name, secret);
    }

    pub fn add_required_attribute(&mut self, attribute: String) {
        self.required_attributes.push(attribute);
    }

    pub fn validate(&self) -> Result<()> {
        if self.role.name.is_empty() {
            return Err(Error::Validation("Role name cannot be empty".to_string()));
        }

        if self.role.capabilities.is_empty() {
            return Err(Error::Validation(
                "Role must have at least one capability".to_string(),
            ));
        }

        if self.required_attributes.is_empty() {
            return Err(Error::Validation(
                "Bundle must have at least one required attribute".to_string(),
            ));
        }

        for (name, secret) in &self.secrets {
            if secret.key.is_empty() {
                return Err(Error::Validation(format!(
                    "Secret '{}' has empty key",
                    name
                )));
            }
        }

        Ok(())
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(Error::Serialization)
    }

    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(Error::Deserialization)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_bundle() {
        let bundle = ConfigurationBundle::new(
            BundleTarget::Role("test-agent".to_string()),
            AgentRole {
                name: "test-agent".to_string(),
                purpose: "Testing".to_string(),
                capabilities: vec!["test".to_string()],
            },
            "admin".to_string(),
            "test".to_string(),
        );

        assert_eq!(bundle.role.name, "test-agent");
        assert_eq!(bundle.metadata.environment, "test");
    }

    #[test]
    fn test_add_setting() {
        let mut bundle = ConfigurationBundle::new(
            BundleTarget::Role("test-agent".to_string()),
            AgentRole {
                name: "test-agent".to_string(),
                purpose: "Testing".to_string(),
                capabilities: vec!["test".to_string()],
            },
            "admin".to_string(),
            "test".to_string(),
        );

        bundle.add_setting("timeout".to_string(), serde_json::json!(30));
        assert_eq!(bundle.settings.get("timeout"), Some(&serde_json::json!(30)));
    }

    #[test]
    fn test_add_entitlement() {
        let mut bundle = ConfigurationBundle::new(
            BundleTarget::Role("test-agent".to_string()),
            AgentRole {
                name: "test-agent".to_string(),
                purpose: "Testing".to_string(),
                capabilities: vec!["test".to_string()],
            },
            "admin".to_string(),
            "test".to_string(),
        );

        bundle.add_entitlement(Entitlement {
            resource_type: ResourceType::McpTool,
            resource_id: "test-tool".to_string(),
            permissions: vec![Permission::Execute],
        });

        assert_eq!(bundle.entitlements.len(), 1);
        assert_eq!(bundle.entitlements[0].resource_id, "test-tool");
    }

    #[test]
    fn test_validation_empty_role() {
        let bundle = ConfigurationBundle::new(
            BundleTarget::Role("test-agent".to_string()),
            AgentRole {
                name: "".to_string(),
                purpose: "Testing".to_string(),
                capabilities: vec!["test".to_string()],
            },
            "admin".to_string(),
            "test".to_string(),
        );

        assert!(bundle.validate().is_err());
    }

    #[test]
    fn test_validation_no_capabilities() {
        let bundle = ConfigurationBundle::new(
            BundleTarget::Role("test-agent".to_string()),
            AgentRole {
                name: "test-agent".to_string(),
                purpose: "Testing".to_string(),
                capabilities: vec![],
            },
            "admin".to_string(),
            "test".to_string(),
        );

        assert!(bundle.validate().is_err());
    }

    #[test]
    fn test_validation_no_attributes() {
        let bundle = ConfigurationBundle::new(
            BundleTarget::Role("test-agent".to_string()),
            AgentRole {
                name: "test-agent".to_string(),
                purpose: "Testing".to_string(),
                capabilities: vec!["test".to_string()],
            },
            "admin".to_string(),
            "test".to_string(),
        );

        assert!(bundle.validate().is_err());
    }

    #[test]
    fn test_validation_success() {
        let mut bundle = ConfigurationBundle::new(
            BundleTarget::Role("test-agent".to_string()),
            AgentRole {
                name: "test-agent".to_string(),
                purpose: "Testing".to_string(),
                capabilities: vec!["test".to_string()],
            },
            "admin".to_string(),
            "test".to_string(),
        );

        bundle.add_required_attribute("agent.role=test-agent".to_string());
        assert!(bundle.validate().is_ok());
    }

    #[test]
    fn test_serialization() {
        let mut bundle = ConfigurationBundle::new(
            BundleTarget::Role("test-agent".to_string()),
            AgentRole {
                name: "test-agent".to_string(),
                purpose: "Testing".to_string(),
                capabilities: vec!["test".to_string()],
            },
            "admin".to_string(),
            "test".to_string(),
        );

        bundle.add_required_attribute("agent.role=test-agent".to_string());
        bundle.add_setting("timeout".to_string(), serde_json::json!(30));

        let json = bundle.to_json().unwrap();
        let deserialized = ConfigurationBundle::from_json(&json).unwrap();

        assert_eq!(bundle.bundle_id, deserialized.bundle_id);
        assert_eq!(bundle.role.name, deserialized.role.name);
        assert_eq!(bundle.settings, deserialized.settings);
    }
}
