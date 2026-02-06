//! TDF Manifest extension methods
//!
//! This module provides extension methods for manifest types from `opentdf-protocol`
//! that require dependencies on Policy and crypto operations.

use crate::policy::{Policy, PolicyError};
use opentdf_crypto::{
    PayloadKey, calculate_policy_binding, calculate_root_signature, verify_root_signature,
};

// Re-export protocol types for backward compatibility
pub use opentdf_protocol::{
    EncryptionInformation, EncryptionMethod, IntegrityInformation, KeyAccess, Payload,
    PolicyBinding, RootSignature, Segment, TdfManifest,
};

/// Extension trait for IntegrityInformation that requires crypto operations
pub trait IntegrityInformationExt {
    fn generate_root_signature(
        &mut self,
        gmac_tags: &[Vec<u8>],
        payload_key: &[u8],
    ) -> Result<(), String>;

    fn verify_root_signature(
        &self,
        gmac_tags: &[Vec<u8>],
        payload_key: &[u8],
    ) -> Result<(), String>;
}

impl IntegrityInformationExt for IntegrityInformation {
    /// Generate root signature from GMAC tags using constant-time HMAC
    ///
    /// The root signature is calculated as:
    /// Base64(HMAC-SHA256(payloadKey, concat(gmac1, gmac2, ...)))
    ///
    /// This uses the constant-time implementation from `opentdf-crypto`
    /// to prevent timing attacks.
    fn generate_root_signature(
        &mut self,
        gmac_tags: &[Vec<u8>],
        payload_key: &[u8],
    ) -> Result<(), String> {
        let payload_key = PayloadKey::from_slice(payload_key)
            .map_err(|e| format!("Invalid payload key: {}", e))?;

        let signature = calculate_root_signature(gmac_tags, &payload_key)
            .map_err(|e| format!("Failed to calculate root signature: {}", e))?;

        self.root_signature.sig = signature;
        self.root_signature.alg = "HS256".to_string();

        Ok(())
    }

    /// Verify root signature against GMAC tags using constant-time comparison
    ///
    /// This validates the integrity of encrypted segments by:
    /// 1. Concatenating all GMAC tags as raw bytes
    /// 2. Calculating HMAC-SHA256 over concatenated tags using payload key
    /// 3. Comparing result with stored root signature in constant time
    ///
    /// Returns Ok(()) if signature is valid, Err otherwise.
    ///
    /// # Security Note
    ///
    /// This uses `subtle::ConstantTimeEq` from the crypto crate to prevent
    /// timing attacks.
    fn verify_root_signature(
        &self,
        gmac_tags: &[Vec<u8>],
        payload_key: &[u8],
    ) -> Result<(), String> {
        let payload_key = PayloadKey::from_slice(payload_key)
            .map_err(|e| format!("Invalid payload key: {}", e))?;

        verify_root_signature(gmac_tags, &payload_key, &self.root_signature.sig)
            .map_err(|e| format!("Signature verification failed: {}", e))
    }
}

/// Extension trait for KeyAccess that requires crypto operations
pub trait KeyAccessExt {
    fn generate_policy_binding_raw(&mut self, policy: &str, key: &[u8]) -> Result<(), String>;

    fn generate_policy_binding(&mut self, policy: &Policy, key: &[u8]) -> Result<(), PolicyError>;
}

impl KeyAccessExt for KeyAccess {
    /// Generate policy binding using HMAC-SHA256 from raw policy string
    ///
    /// This matches the OpenTDF Go SDK format:
    /// 1. Base64 encode the policy JSON
    /// 2. HMAC-SHA256 the base64-encoded policy using the key
    /// 3. Hex encode the HMAC result (32 bytes → 64 hex chars)
    /// 4. Base64 encode the hex string for storage
    fn generate_policy_binding_raw(&mut self, policy: &str, key: &[u8]) -> Result<(), String> {
        let binding = calculate_policy_binding(policy, key)
            .map_err(|e| format!("Failed to calculate policy binding: {}", e))?;

        self.policy_binding.hash = binding;
        self.policy_binding.alg = "HS256".to_string();
        Ok(())
    }

    /// Generate policy binding using HMAC-SHA256 from a Policy object
    fn generate_policy_binding(&mut self, policy: &Policy, key: &[u8]) -> Result<(), PolicyError> {
        let policy_json = policy.to_json()?;
        self.generate_policy_binding_raw(&policy_json, key)
            .map_err(|e| PolicyError::EvaluationError {
                reason: format!("Failed to generate policy binding: {}", e),
                attribute: None,
            })?;
        Ok(())
    }
}

/// Extension trait for TdfManifest that requires Policy operations
pub trait TdfManifestExt {
    fn set_policy(&mut self, policy: &Policy) -> Result<(), PolicyError>;
    fn get_policy(&self) -> Result<Policy, PolicyError>;
}

impl TdfManifestExt for TdfManifest {
    /// Set the policy for the manifest using a Policy object
    fn set_policy(&mut self, policy: &Policy) -> Result<(), PolicyError> {
        let policy_json = policy.to_json()?;
        self.set_policy_raw(&policy_json);
        Ok(())
    }

    /// Get the policy from the manifest as a Policy object
    fn get_policy(&self) -> Result<Policy, PolicyError> {
        let policy_json = match self.get_policy_raw() {
            Ok(json) => json,
            Err(e) => {
                return Err(PolicyError::SerializationError(serde_json::Error::io(
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()),
                )));
            }
        };
        Policy::from_json(&policy_json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_access_metadata() {
        let mut key_access = KeyAccess::new("http://example.com".to_string());

        assert!(key_access.encrypted_metadata.is_none());
        key_access.set_encrypted_metadata("test metadata");
        assert!(key_access.encrypted_metadata.is_some());
        key_access.clear_encrypted_metadata();
        assert!(key_access.encrypted_metadata.is_none());
    }

    #[test]
    fn test_key_access_policy_binding_raw() {
        use crate::manifest::KeyAccessExt;

        let mut key_access = KeyAccess::new("http://kas.example.com:4000".to_string());
        let policy = r#"{"uuid":"test","body":{"attributes":[],"dissem":["user@example.com"]}}"#;
        let key = b"test-key-for-hmac";

        key_access.generate_policy_binding_raw(policy, key).unwrap();
        assert!(!key_access.policy_binding.hash.is_empty());
        assert_eq!(key_access.policy_binding.alg, "HS256");
    }

    #[test]
    fn test_key_access_policy_binding_object() {
        use crate::manifest::KeyAccessExt;
        use crate::policy::{Policy, PolicyBody};

        let mut key_access = KeyAccess::new("http://kas.example.com:4000".to_string());
        let policy = Policy {
            uuid: "test".to_string(),
            valid_from: None,
            valid_to: None,
            body: PolicyBody {
                attributes: vec![],
                dissem: vec!["user@example.com".to_string()],
            },
        };
        let key = b"test-key-for-hmac";

        key_access.generate_policy_binding(&policy, key).unwrap();
        assert!(!key_access.policy_binding.hash.is_empty());
        assert_eq!(key_access.policy_binding.alg, "HS256");
    }

    #[test]
    fn test_manifest_serialization() {
        let manifest = TdfManifest::new(
            "0.payload".to_string(),
            "http://kas.example.com:4000".to_string(),
        );

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let deserialized: TdfManifest = serde_json::from_str(&json).unwrap();

        assert_eq!(manifest.payload.url, deserialized.payload.url);
        assert_eq!(
            manifest.encryption_information.key_access[0].url,
            deserialized.encryption_information.key_access[0].url
        );
    }

    #[test]
    fn test_policy_raw_encoding() {
        let mut manifest = TdfManifest::new(
            "0.payload".to_string(),
            "http://kas.example.com:4000".to_string(),
        );

        let policy = r#"{"uuid":"test","body":{"attributes":[],"dissem":["user@example.com"]}}"#;
        manifest.set_policy_raw(policy);

        let decoded_policy = manifest.get_policy_raw().unwrap();
        assert_eq!(policy, decoded_policy);
    }

    #[test]
    fn test_policy_object_encoding() {
        use crate::manifest::TdfManifestExt;
        use crate::policy::{Policy, PolicyBody};

        let mut manifest = TdfManifest::new(
            "0.payload".to_string(),
            "http://kas.example.com:4000".to_string(),
        );

        let policy = Policy {
            uuid: "test".to_string(),
            valid_from: None,
            valid_to: None,
            body: PolicyBody {
                attributes: vec![],
                dissem: vec!["user@example.com".to_string()],
            },
        };

        manifest.set_policy(&policy).unwrap();

        let retrieved_policy = manifest.get_policy().unwrap();
        assert_eq!(policy.uuid, retrieved_policy.uuid);
        assert_eq!(policy.body.dissem, retrieved_policy.body.dissem);
    }
}
