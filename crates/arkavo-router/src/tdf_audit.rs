//! TDF audit encryption for cloud-bound LLM messages.
//!
//! Encrypts prompts using OpenTDF (AES-256-GCM, ZTDF-JSON format) before they are
//! sent to cloud LLM providers. The encrypted manifests serve as a local audit trail
//! proving what was sent, while the original plaintext is still sent for functional
//! correctness (Phase 1). A future Phase 2 will support ciphertext-only transmission
//! via a TDF-aware proxy.

use arkavo_llm::{Message, Role};
use arkavo_tdf::{
    OpenTdfConfig, OpenTdfService, PolicyBuilder, TdfEncryptor, TdfManifest, arkavo_attrs,
};

/// Configuration for TDF message encryption audit.
///
/// Each agent in the mesh is its own KAS. The `kas_url` points to the local
/// agent's own A2A endpoint (e.g., `http://localhost:8360`), not a cloud service.
/// The Orchestrator agent serves as the central KAS for the mesh, but any agent
/// can potentially serve as a KAS for its own encrypted payloads.
#[derive(Debug, Clone)]
pub struct TdfAuditConfig {
    /// Local agent KAS URL (e.g., `http://localhost:8360`).
    /// Each agent is its own KAS -- no external SaaS dependency.
    pub kas_url: String,
    /// Agent identifier bound into the TDF policy.
    pub agent_id: String,
    /// Key identifier from SwarmKit `kas.key_id`.
    pub key_id: String,
}

/// Encrypts cloud-bound LLM messages and produces TDF audit manifests.
///
/// Phase 1: audit mode -- encrypts messages and returns manifests for logging.
/// The original plaintext is still sent to the cloud provider.
pub struct MessageEncryptor {
    service: OpenTdfService,
    config: TdfAuditConfig,
}

impl MessageEncryptor {
    /// Create a new encryptor from audit config.
    #[must_use]
    pub fn new(config: TdfAuditConfig) -> Self {
        let service = OpenTdfService::new(OpenTdfConfig::new(&config.kas_url));
        Self { service, config }
    }

    /// Encrypt user and system messages, returning index+manifest pairs.
    ///
    /// Only `User` and `System` role messages are encrypted (assistant responses
    /// and tool results are not outbound prompts). Encryption errors are logged
    /// but never propagated -- audit is best-effort and must not block LLM calls.
    pub async fn encrypt_messages(&self, messages: &[Message]) -> Vec<(usize, TdfManifest)> {
        let policy = self.build_policy();
        let mut results = Vec::new();

        for (idx, msg) in messages.iter().enumerate() {
            if !matches!(msg.role, Role::User | Role::System) {
                continue;
            }

            if msg.content.is_empty() {
                continue;
            }

            match self.service.encrypt(msg.content.as_bytes(), &policy).await {
                Ok(manifest) => results.push((idx, manifest)),
                Err(e) => {
                    tracing::warn!(
                        message_index = idx,
                        error = %e,
                        "TDF audit encryption failed for message, skipping"
                    );
                }
            }
        }

        results
    }

    /// Build the ABAC policy for cloud-bound prompt encryption.
    fn build_policy(&self) -> arkavo_tdf::Policy {
        PolicyBuilder::new()
            .attribute_single(arkavo_attrs::AGENT_ID, &self.config.agent_id)
            .attribute_single(arkavo_attrs::ROLE, "cloud-prompt")
            .build()
            .unwrap_or_else(|_| {
                PolicyBuilder::new()
                    .attribute_single(arkavo_attrs::ROLE, "cloud-prompt")
                    .build_unchecked()
            })
    }

    /// Get the agent ID from config.
    #[must_use]
    pub fn agent_id(&self) -> &str {
        &self.config.agent_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> TdfAuditConfig {
        TdfAuditConfig {
            kas_url: "https://kas.example.com".to_string(),
            agent_id: "test-agent".to_string(),
            key_id: "test-key-1".to_string(),
        }
    }

    fn user_message(content: &str) -> Message {
        Message {
            role: Role::User,
            content: content.to_string(),
            images: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        }
    }

    fn system_message(content: &str) -> Message {
        Message {
            role: Role::System,
            content: content.to_string(),
            images: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        }
    }

    fn assistant_message(content: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: content.to_string(),
            images: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        }
    }

    #[tokio::test]
    async fn encrypt_single_user_message() {
        let encryptor = MessageEncryptor::new(test_config());
        let messages = vec![user_message("What is the meaning of life?")];

        let results = encryptor.encrypt_messages(&messages).await;

        assert_eq!(results.len(), 1);
        let (idx, manifest) = &results[0];
        assert_eq!(*idx, 0);
        assert_eq!(manifest.version, "3.0.0");
        assert_eq!(
            manifest.encryption_information.method.algorithm,
            "AES-256-GCM"
        );
        assert!(!manifest.payload.value.is_empty());
    }

    #[tokio::test]
    async fn encrypt_skips_non_user_messages() {
        let encryptor = MessageEncryptor::new(test_config());
        let messages = vec![
            user_message("Hello"),
            assistant_message("Hi there"),
            user_message("How are you?"),
        ];

        let results = encryptor.encrypt_messages(&messages).await;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 0);
        assert_eq!(results[1].0, 2);
    }

    #[tokio::test]
    async fn encrypt_empty_messages() {
        let encryptor = MessageEncryptor::new(test_config());
        let results = encryptor.encrypt_messages(&[]).await;
        assert!(results.is_empty());
    }

    #[test]
    fn policy_contains_agent_id() {
        let encryptor = MessageEncryptor::new(test_config());
        let policy = encryptor.build_policy();

        assert_eq!(policy.attributes.len(), 2);
        let agent_attr = policy
            .attributes
            .iter()
            .find(|a| a.attribute == arkavo_attrs::AGENT_ID)
            .expect("policy should contain AGENT_ID attribute");
        assert_eq!(agent_attr.values, vec!["test-agent"]);
    }

    #[tokio::test]
    async fn encrypt_multiple_messages() {
        let encryptor = MessageEncryptor::new(test_config());
        let messages = vec![
            system_message("You are helpful"),
            user_message("First question"),
            user_message("Second question"),
        ];

        let results = encryptor.encrypt_messages(&messages).await;

        assert_eq!(results.len(), 3);
        // All manifests should have different ciphertext
        assert_ne!(results[0].1.payload.value, results[1].1.payload.value);
    }

    #[test]
    fn config_propagation() {
        let config = TdfAuditConfig {
            kas_url: "https://custom-kas.example.com".to_string(),
            agent_id: "my-agent".to_string(),
            key_id: "my-key".to_string(),
        };
        let encryptor = MessageEncryptor::new(config);
        assert_eq!(encryptor.agent_id(), "my-agent");
    }
}
