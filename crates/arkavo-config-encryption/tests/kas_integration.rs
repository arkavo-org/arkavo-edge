//! Integration test for KAS-based encryption and decryption
//!
//! This test demonstrates the full encrypt/decrypt cycle using the Arkavo KAS.
//! It requires the 'kas' feature flag to be enabled.
//!
//! To run: cargo test --features kas --test kas_integration -- --ignored

#[cfg(feature = "kas")]
#[cfg(test)]
mod kas_tests {
    use arkavo_config_bundle::{AgentRole, BundleTarget, ConfigurationBundle};
    use arkavo_config_encryption::{
        AgentCredential, ConfigBundleDecryptor, ConfigBundleEncryptor, KasConfig, Policy,
        PolicyAttribute,
    };
    use std::collections::HashMap;

    /// Test encryption with production KAS URL
    #[test]
    fn test_encrypt_with_production_kas() {
        let mut bundle = ConfigurationBundle::new(
            BundleTarget::Role("test-agent".to_string()),
            AgentRole {
                name: "test-agent".to_string(),
                purpose: "Integration testing".to_string(),
                capabilities: vec!["testing".to_string()],
            },
            "test-orchestrator".to_string(),
            "test".to_string(),
        );

        bundle.add_setting("test_key".to_string(), serde_json::json!("test_value"));
        bundle.add_required_attribute("agent.role=test-agent".to_string());

        let kas = KasConfig::production();
        let encryptor = ConfigBundleEncryptor::new(kas.rewrap_url().to_string());

        let policy = Policy {
            attributes: vec![PolicyAttribute {
                attribute: "data/clearance".to_string(),
                value: "internal".to_string(),
                display_name: "Data Clearance".to_string(),
            }],
            dissemination: vec!["agent.role=test-agent".to_string()],
        };

        let result = encryptor.encrypt_bundle(&bundle, policy);
        assert!(result.is_ok());

        let encrypted = result.unwrap();
        assert_eq!(encrypted.bundle_id, bundle.bundle_id);
        assert!(!encrypted.encrypted_data.is_empty());
        assert_eq!(
            encrypted.policy_manifest.kas_url,
            "https://platform.arkavo.net/kas/v2/rewrap"
        );
    }

    /// Test that decryption without KAS client fails appropriately
    #[test]
    fn test_decrypt_without_kas_fails() {
        let mut bundle = ConfigurationBundle::new(
            BundleTarget::Role("test-agent".to_string()),
            AgentRole {
                name: "test-agent".to_string(),
                purpose: "Integration testing".to_string(),
                capabilities: vec!["testing".to_string()],
            },
            "test-orchestrator".to_string(),
            "test".to_string(),
        );

        bundle.add_required_attribute("agent.role=test-agent".to_string());

        let kas = KasConfig::production();
        let encryptor = ConfigBundleEncryptor::new(kas.rewrap_url().to_string());

        let policy = Policy {
            attributes: vec![PolicyAttribute {
                attribute: "data/clearance".to_string(),
                value: "internal".to_string(),
                display_name: "Data Clearance".to_string(),
            }],
            dissemination: vec!["agent.role=test-agent".to_string()],
        };

        let encrypted = encryptor.encrypt_bundle(&bundle, policy).unwrap();

        let mut attributes = HashMap::new();
        attributes.insert("agent.role".to_string(), "test-agent".to_string());

        let credential = AgentCredential::new("test-agent-001".to_string(), attributes).unwrap();
        let decryptor = ConfigBundleDecryptor::new(credential);

        // Synchronous decrypt should fail with helpful message
        let result = decryptor.decrypt_bundle(&encrypted);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("KAS integration"));
    }

    /// Test async decryption with KAS client (requires OAuth token)
    /// This test is ignored by default as it requires network access and credentials
    #[tokio::test]
    #[ignore]
    async fn test_decrypt_with_kas_client() {
        // This test requires:
        // 1. ARKAVO_KAS_TOKEN environment variable with valid OAuth token
        // 2. Network access to https://platform.arkavo.net

        let token = std::env::var("ARKAVO_KAS_TOKEN")
            .expect("ARKAVO_KAS_TOKEN environment variable required for KAS integration test");

        let mut bundle = ConfigurationBundle::new(
            BundleTarget::Role("integration-test-agent".to_string()),
            AgentRole {
                name: "integration-test-agent".to_string(),
                purpose: "Integration testing".to_string(),
                capabilities: vec!["testing".to_string()],
            },
            "test-orchestrator".to_string(),
            "test".to_string(),
        );

        bundle.add_setting("test_key".to_string(), serde_json::json!("test_value"));
        bundle.add_required_attribute("agent.role=integration-test-agent".to_string());
        bundle.add_required_attribute("environment=test".to_string());

        // Encrypt
        let kas = KasConfig::production();
        let encryptor = ConfigBundleEncryptor::new(kas.rewrap_url().to_string());

        let policy = Policy {
            attributes: vec![
                PolicyAttribute {
                    attribute: "data/clearance".to_string(),
                    value: "internal".to_string(),
                    display_name: "Data Clearance".to_string(),
                },
                PolicyAttribute {
                    attribute: "agent/identity".to_string(),
                    value: "autonomous_service".to_string(),
                    display_name: "Agent Identity".to_string(),
                },
            ],
            dissemination: vec![
                "agent.role=integration-test-agent".to_string(),
                "environment=test".to_string(),
            ],
        };

        let encrypted = encryptor.encrypt_bundle(&bundle, policy).unwrap();

        println!("✓ Encrypted bundle created");
        println!("  Bundle ID: {}", encrypted.bundle_id);
        println!("  Policy ID: {}", encrypted.policy_manifest.policy_id);
        println!("  Encrypted size: {} bytes", encrypted.encrypted_data.len());

        // Decrypt with KAS
        let mut attributes = HashMap::new();
        attributes.insert(
            "agent.role".to_string(),
            "integration-test-agent".to_string(),
        );
        attributes.insert("environment".to_string(), "test".to_string());

        let credential = AgentCredential::new("test-agent-001".to_string(), attributes).unwrap();
        let decryptor = ConfigBundleDecryptor::new(credential);

        // Create KAS client
        let kas_client =
            opentdf::KasClient::new(kas.rewrap_url(), &token).expect("Failed to create KAS client");

        println!("✓ KAS client created");
        println!("  KAS URL: {}", kas.rewrap_url());

        // Decrypt
        let result = decryptor.decrypt_bundle_async(&encrypted, kas_client).await;

        match result {
            Ok(decrypted) => {
                println!("✓ Decryption successful!");
                println!("  Bundle ID: {}", decrypted.bundle_id);
                println!("  Role: {}", decrypted.role.name);
                println!("  Settings: {:?}", decrypted.settings);

                assert_eq!(decrypted.bundle_id, bundle.bundle_id);
                assert_eq!(decrypted.role.name, bundle.role.name);
                assert_eq!(
                    decrypted.settings.get("test_key"),
                    Some(&serde_json::json!("test_value"))
                );
            }
            Err(e) => {
                panic!("Decryption failed: {}", e);
            }
        }
    }

    /// Test attribute verification during decryption
    #[test]
    fn test_attribute_verification() {
        let mut bundle = ConfigurationBundle::new(
            BundleTarget::Role("production-agent".to_string()),
            AgentRole {
                name: "production-agent".to_string(),
                purpose: "Production workload".to_string(),
                capabilities: vec!["production".to_string()],
            },
            "orchestrator".to_string(),
            "production".to_string(),
        );

        bundle.add_required_attribute("agent.role=production-agent".to_string());
        bundle.add_required_attribute("environment=production".to_string());

        let kas = KasConfig::production();
        let encryptor = ConfigBundleEncryptor::new(kas.rewrap_url().to_string());

        let policy = Policy {
            attributes: vec![
                PolicyAttribute {
                    attribute: "data/clearance".to_string(),
                    value: "confidential".to_string(),
                    display_name: "Data Clearance".to_string(),
                },
                PolicyAttribute {
                    attribute: "capability/filesystem".to_string(),
                    value: "read_content".to_string(),
                    display_name: "Filesystem Capability".to_string(),
                },
            ],
            dissemination: vec![
                "agent.role=production-agent".to_string(),
                "environment=production".to_string(),
            ],
        };

        let encrypted = encryptor.encrypt_bundle(&bundle, policy).unwrap();

        // Try to decrypt with wrong attributes (missing environment=production)
        let mut wrong_attributes = HashMap::new();
        wrong_attributes.insert("agent.role".to_string(), "production-agent".to_string());
        wrong_attributes.insert("environment".to_string(), "development".to_string());

        let wrong_identity =
            AgentCredential::new("wrong-agent-001".to_string(), wrong_attributes).unwrap();
        let decryptor = ConfigBundleDecryptor::new(wrong_identity);

        let result = decryptor.decrypt_bundle(&encrypted);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("does not have required attributes")
        );
    }
}

#[cfg(not(feature = "kas"))]
#[cfg(test)]
mod no_kas_tests {
    #[test]
    fn test_kas_feature_not_enabled() {
        // This test passes when KAS feature is not enabled
        assert!(true);
    }
}
