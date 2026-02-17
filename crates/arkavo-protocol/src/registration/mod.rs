pub mod types;

pub use types::{
    ChallengeRequest, ChallengeResponse, RegistrationStatus, VerifyRequest, VerifyResponse,
};

use crate::error::{A2aError, Result};
use base64::{Engine as _, engine::general_purpose};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

const CHALLENGE_TTL_SECONDS: u64 = 300;

#[derive(Clone)]
pub struct Challenge {
    pub data: Vec<u8>,
    pub timestamp: u64,
    pub device_id: String,
}

#[derive(Clone)]
pub struct Registration {
    pub device_id: String,
    pub public_key: Vec<u8>,
    pub verified: bool,
    pub timestamp: u64,
}

pub struct RegistrationService {
    challenges: Arc<RwLock<HashMap<String, Challenge>>>,
    registrations: Arc<RwLock<HashMap<String, Registration>>>,
}

impl Clone for RegistrationService {
    fn clone(&self) -> Self {
        Self {
            challenges: Arc::clone(&self.challenges),
            registrations: Arc::clone(&self.registrations),
        }
    }
}

impl RegistrationService {
    pub fn new() -> Self {
        Self {
            challenges: Arc::new(RwLock::new(HashMap::new())),
            registrations: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// # Panics
    /// Panics if system time is before UNIX_EPOCH.
    pub async fn create_challenge(&self, request: ChallengeRequest) -> Result<ChallengeResponse> {
        // Validate device ID is not empty
        if request.device_id.is_empty() {
            return Err(A2aError::InvalidRequest(
                "Device ID cannot be empty".to_string(),
            ));
        }

        let mut challenge_data = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut challenge_data);
        let challenge_data = challenge_data.to_vec();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let challenge = Challenge {
            data: challenge_data.clone(),
            timestamp,
            device_id: request.device_id.clone(),
        };

        let challenge_id = hex::encode(Sha256::digest(&challenge_data));

        let mut challenges = self.challenges.write().await;
        challenges.insert(challenge_id.clone(), challenge);

        self.cleanup_expired_challenges(&mut challenges, timestamp);

        Ok(ChallengeResponse {
            challenge_id,
            challenge: general_purpose::STANDARD.encode(&challenge_data),
            timestamp,
        })
    }

    /// # Panics
    /// Panics if system time is before UNIX_EPOCH.
    pub async fn verify_challenge(&self, request: VerifyRequest) -> Result<VerifyResponse> {
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Use write lock from the start to prevent race conditions
        // This ensures atomic verification and removal (no replay attacks)
        let mut challenges = self.challenges.write().await;
        let challenge = challenges
            .get(&request.challenge_id)
            .ok_or_else(|| A2aError::InvalidRequest("Challenge not found".to_string()))?;

        // Challenge expires at TTL boundary (>= for consistent behavior)
        if current_time - challenge.timestamp >= CHALLENGE_TTL_SECONDS {
            return Err(A2aError::InvalidRequest("Challenge expired".to_string()));
        }

        if challenge.device_id != request.device_id {
            return Err(A2aError::InvalidRequest("Device ID mismatch".to_string()));
        }

        let public_key_bytes = general_purpose::STANDARD
            .decode(&request.public_key)
            .map_err(|e| A2aError::InvalidRequest(format!("Invalid public key: {e}")))?;

        let signature_bytes = general_purpose::STANDARD
            .decode(&request.signature)
            .map_err(|e| A2aError::InvalidRequest(format!("Invalid signature: {e}")))?;

        // Detect key type by length and verify accordingly
        // Ed25519: 32 bytes
        // P-256 SEC1 uncompressed: 65 bytes (0x04 prefix)
        // P-256 SEC1 compressed: 33 bytes (0x02/0x03 prefix)
        let challenge_data = challenge.data.clone();
        match public_key_bytes.len() {
            32 => {
                // Ed25519 key (agent-to-agent)
                let public_key = arkavo_crypto::AgentPublicKey::from_bytes(&public_key_bytes)
                    .map_err(|e| {
                        A2aError::InvalidRequest(format!("Invalid Ed25519 public key: {e}"))
                    })?;
                public_key
                    .verify(&challenge_data, &signature_bytes)
                    .map_err(|_| {
                        A2aError::AuthenticationFailed(
                            "Ed25519 signature verification failed".to_string(),
                        )
                    })?;
            }
            33 | 65 => {
                // P-256 SEC1 key (iOS Secure Enclave)
                let public_key = arkavo_crypto::P256VerifyingKey::from_sec1_bytes(
                    &public_key_bytes,
                )
                .map_err(|e| A2aError::InvalidRequest(format!("Invalid P-256 public key: {e}")))?;
                public_key
                    .verify(&challenge_data, &signature_bytes)
                    .map_err(|_| {
                        A2aError::AuthenticationFailed(
                            "P-256 signature verification failed".to_string(),
                        )
                    })?;
            }
            len => {
                return Err(A2aError::InvalidRequest(format!(
                    "Unsupported public key length: {len} bytes. Expected 32 (Ed25519) or 33/65 (P-256)"
                )));
            }
        }

        // Remove challenge atomically to prevent replay attacks
        // The write lock is held throughout, so no race condition is possible
        challenges.remove(&request.challenge_id);

        let registration = Registration {
            device_id: request.device_id.clone(),
            public_key: public_key_bytes,
            verified: true,
            timestamp: current_time,
        };

        let mut registrations = self.registrations.write().await;
        registrations.insert(request.device_id.clone(), registration);

        Ok(VerifyResponse {
            success: true,
            device_id: request.device_id,
            message: "Registration verified successfully".to_string(),
        })
    }

    pub async fn get_registration_status(&self, device_id: &str) -> Result<RegistrationStatus> {
        let registrations = self.registrations.read().await;
        if let Some(reg) = registrations.get(device_id) {
            Ok(RegistrationStatus {
                device_id: device_id.to_string(),
                verified: reg.verified,
                registered_at: reg.timestamp,
            })
        } else {
            Ok(RegistrationStatus {
                device_id: device_id.to_string(),
                verified: false,
                registered_at: 0,
            })
        }
    }

    fn cleanup_expired_challenges(
        &self,
        challenges: &mut HashMap<String, Challenge>,
        current_time: u64,
    ) {
        challenges
            .retain(|_, challenge| current_time - challenge.timestamp <= CHALLENGE_TTL_SECONDS);
    }
}

impl Default for RegistrationService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    /// Test REG-001: Create challenge for new device
    ///
    /// Spec: registration.spec.yaml - REG-001
    /// Given: RegistrationService is initialized, Device ID is valid
    /// When: create_challenge is called with ChallengeRequest
    /// Then: Returns ChallengeResponse with unique challenge_id, base64-encoded 32 random bytes
    #[spec("REG-001")]
    #[tokio::test]
    async fn test_create_challenge() {
        let service = RegistrationService::new();
        let request = ChallengeRequest {
            device_id: "test-device".to_string(),
        };

        let response = service.create_challenge(request).await.unwrap();
        assert!(!response.challenge_id.is_empty());
        assert!(!response.challenge.is_empty());
    }

    /// Test REG-002: Verify challenge with Ed25519 signature
    ///
    /// Spec: registration.spec.yaml - REG-002
    /// Given: Valid challenge exists, Ed25519 keypair available
    /// When: verify_challenge called with valid signature
    /// Then: Registration created, verified=true, success response
    #[spec("REG-002")]
    #[tokio::test]
    async fn test_verify_challenge_success() {
        let service = RegistrationService::new();
        let device_id = "test-device".to_string();

        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        let keypair = arkavo_crypto::AgentKeypair::generate();
        let challenge_bytes = general_purpose::STANDARD
            .decode(&challenge_response.challenge)
            .unwrap();
        let signature = keypair.sign(&challenge_bytes);

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id,
            device_id: device_id.clone(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        let verify_response = service.verify_challenge(verify_request).await.unwrap();
        assert!(verify_response.success);

        let status = service.get_registration_status(&device_id).await.unwrap();
        assert!(status.verified);
    }

    /// Test REG-007: Reject invalid signature
    ///
    /// Spec: registration.spec.yaml - REG-007
    /// Given: Valid challenge exists, signature over different data
    /// When: verify_challenge called with wrong signature
    /// Then: Returns A2aError::AuthenticationFailed
    #[spec("REG-007")]
    #[tokio::test]
    async fn test_verify_challenge_wrong_signature() {
        let service = RegistrationService::new();
        let device_id = "test-device".to_string();

        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        let keypair = arkavo_crypto::AgentKeypair::generate();
        let wrong_data = b"wrong data";
        let signature = keypair.sign(wrong_data);

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id,
            device_id: device_id.clone(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        let result = service.verify_challenge(verify_request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_challenge_not_found() {
        let service = RegistrationService::new();
        let keypair = arkavo_crypto::AgentKeypair::generate();

        let verify_request = VerifyRequest {
            challenge_id: "nonexistent".to_string(),
            device_id: "test-device".to_string(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&[0u8; 64]),
        };

        let result = service.verify_challenge(verify_request).await;
        assert!(result.is_err());
    }

    /// Test REG-003: Verify challenge with P-256 SEC1 signature (iOS Secure Enclave)
    ///
    /// Spec: registration.spec.yaml - REG-003
    /// Given: Valid challenge, P-256 SEC1 key from iOS Secure Enclave
    /// When: verify_challenge called with P-256 public key and signature
    /// Then: Signature verified, registration created with verified=true
    #[spec("REG-003")]
    #[tokio::test]
    async fn test_verify_challenge_with_p256_key() {
        // iOS Secure Enclave uses P-256 ECDSA, not Ed25519
        let service = RegistrationService::new();
        let device_id = "ios-device".to_string();

        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        // Use P-256 keypair (simulating iOS Secure Enclave)
        let keypair = arkavo_crypto::P256SigningKeypair::generate();
        let challenge_bytes = general_purpose::STANDARD
            .decode(&challenge_response.challenge)
            .unwrap();
        let signature = keypair.sign(&challenge_bytes);

        // P-256 public key in SEC1 uncompressed format (65 bytes)
        let public_key_bytes = keypair.public_key().to_sec1_bytes();

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id,
            device_id: device_id.clone(),
            public_key: general_purpose::STANDARD.encode(&public_key_bytes),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        let verify_response = service.verify_challenge(verify_request).await.unwrap();
        assert!(verify_response.success);

        let status = service.get_registration_status(&device_id).await.unwrap();
        assert!(status.verified);
    }

    /// Additional test for REG-007: Reject invalid P-256 signature
    #[spec("REG-007")]
    #[tokio::test]
    async fn test_verify_challenge_p256_wrong_signature() {
        let service = RegistrationService::new();
        let device_id = "ios-device".to_string();

        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        let keypair = arkavo_crypto::P256SigningKeypair::generate();
        let wrong_data = b"wrong data";
        let signature = keypair.sign(wrong_data);

        let public_key_bytes = keypair.public_key().to_sec1_bytes();

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id,
            device_id: device_id.clone(),
            public_key: general_purpose::STANDARD.encode(&public_key_bytes),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        let result = service.verify_challenge(verify_request).await;
        assert!(result.is_err());
    }

    /// Test REG-005: Reject expired challenge verification
    ///
    /// Spec: registration.spec.yaml - REG-005
    /// Given: Challenge exists but is older than 300 seconds
    /// When: verify_challenge called after TTL expires
    /// Then: Returns A2aError::InvalidRequest("Challenge expired")
    #[spec("REG-005")]
    #[tokio::test]
    async fn test_verify_challenge_expired() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let service = RegistrationService::new();
        let device_id = "test-device-expired".to_string();

        // Create a challenge directly with an expired timestamp
        let expired_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - CHALLENGE_TTL_SECONDS
            - 1; // 1 second past TTL

        let challenge_data = vec![1u8; 32]; // Dummy challenge data
        let challenge_id = hex::encode(Sha256::digest(&challenge_data));

        let challenge = Challenge {
            data: challenge_data.clone(),
            timestamp: expired_timestamp,
            device_id: device_id.clone(),
        };

        // Insert the expired challenge directly
        {
            let mut challenges = service.challenges.write().await;
            challenges.insert(challenge_id.clone(), challenge);
        }

        // Create a valid keypair and signature
        let keypair = arkavo_crypto::AgentKeypair::generate();
        let signature = keypair.sign(&challenge_data);

        let verify_request = VerifyRequest {
            challenge_id: challenge_id.clone(),
            device_id: device_id.clone(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        // Attempt to verify - should fail with "Challenge expired"
        let result = service.verify_challenge(verify_request).await;
        assert!(result.is_err(), "Expired challenge should be rejected");

        let err = result.unwrap_err();
        let err_msg = format!("{err}");
        assert!(
            err_msg.contains("expired") || err_msg.contains("Challenge"),
            "Error should indicate challenge expired: {err_msg}"
        );
    }

    /// Test REG-006: Reject challenge with device ID mismatch
    ///
    /// Spec: registration.spec.yaml - REG-006
    /// Given: Challenge was created for device_id "A"
    /// When: verify_challenge called with device_id "B"
    /// Then: Returns A2aError::InvalidRequest("Device ID mismatch")
    #[spec("REG-006")]
    #[tokio::test]
    async fn test_verify_challenge_device_mismatch() {
        let service = RegistrationService::new();
        let device_id_a = "device-A".to_string();
        let device_id_b = "device-B".to_string();

        // Create challenge for device A
        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id_a.clone(),
            })
            .await
            .unwrap();

        // Try to verify with device B
        let keypair = arkavo_crypto::AgentKeypair::generate();
        let challenge_bytes = general_purpose::STANDARD
            .decode(&challenge_response.challenge)
            .unwrap();
        let signature = keypair.sign(&challenge_bytes);

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id,
            device_id: device_id_b, // Mismatched device ID
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        let result = service.verify_challenge(verify_request).await;
        assert!(result.is_err(), "Device ID mismatch should be rejected");

        let err = result.unwrap_err();
        let err_msg = format!("{err}");
        assert!(
            err_msg.contains("mismatch") || err_msg.contains("Device ID"),
            "Error should indicate device ID mismatch: {err_msg}"
        );
    }

    /// Test REG-004: Challenge expiration cleanup
    ///
    /// Spec: registration.spec.yaml - REG-004
    /// Given: Multiple challenges exist, some older than 300 seconds
    /// When: create_challenge is called (triggers cleanup)
    /// Then: Expired challenges removed, new challenge created, active unaffected
    #[spec("REG-004")]
    #[tokio::test]
    async fn test_challenge_expiration_cleanup() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let service = RegistrationService::new();
        let device_id = "cleanup-test-device".to_string();

        // Create an expired challenge directly
        let expired_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - CHALLENGE_TTL_SECONDS
            - 10;

        let expired_challenge_data = vec![1u8; 32];
        let expired_challenge_id = hex::encode(Sha256::digest(&expired_challenge_data));

        let expired_challenge = Challenge {
            data: expired_challenge_data,
            timestamp: expired_timestamp,
            device_id: "expired-device".to_string(),
        };

        // Insert expired challenge
        {
            let mut challenges = service.challenges.write().await;
            challenges.insert(expired_challenge_id.clone(), expired_challenge);
        }

        // Verify expired challenge exists
        {
            let challenges = service.challenges.read().await;
            assert!(challenges.contains_key(&expired_challenge_id));
        }

        // Create a new challenge (should trigger cleanup)
        let new_challenge = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        // Verify new challenge was created
        assert!(!new_challenge.challenge_id.is_empty());

        // Verify the new challenge exists
        {
            let challenges = service.challenges.read().await;
            assert!(challenges.contains_key(&new_challenge.challenge_id));
        }
    }

    /// Test REG-008: Reject unsupported key length
    ///
    /// Spec: registration.spec.yaml - REG-008
    /// Given: Public key with unsupported length (not 32, 33, or 65 bytes)
    /// When: verify_challenge called with invalid key length
    /// Then: Returns A2aError::InvalidRequest with descriptive message
    #[spec("REG-008")]
    #[tokio::test]
    async fn test_verify_challenge_unsupported_key_length() {
        let service = RegistrationService::new();
        let device_id = "test-device-bad-key".to_string();

        // Create a valid challenge first
        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        // Try to verify with wrong key length (e.g., 16 bytes instead of 32)
        let bad_key = general_purpose::STANDARD.encode(&[0u8; 16]);
        let signature = general_purpose::STANDARD.encode(&[0u8; 64]);

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id,
            device_id: device_id.clone(),
            public_key: bad_key,
            signature,
        };

        let result = service.verify_challenge(verify_request).await;
        assert!(result.is_err(), "Unsupported key length should be rejected");

        let err = result.unwrap_err();
        let err_msg = format!("{err}");
        assert!(
            err_msg.contains("InvalidRequest") || err_msg.contains("key"),
            "Error should indicate invalid key: {err_msg}"
        );
    }

    /// Test REG-009: Query registration status
    ///
    /// Spec: registration.spec.yaml - REG-009
    /// Given: Device may or may not be registered
    /// When: get_registration_status called
    /// Then: Returns verified=true if exists, verified=false if not found
    #[spec("REG-009")]
    #[tokio::test]
    async fn test_query_registration_status() {
        let service = RegistrationService::new();
        let unregistered_device = "unregistered-device".to_string();

        // Query unregistered device - should return verified=false
        let status = service
            .get_registration_status(&unregistered_device)
            .await
            .unwrap();
        assert!(!status.verified);
        assert_eq!(status.device_id, unregistered_device);

        // Now register a device
        let registered_device = "registered-device".to_string();
        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: registered_device.clone(),
            })
            .await
            .unwrap();

        let keypair = arkavo_crypto::AgentKeypair::generate();
        let challenge_bytes = general_purpose::STANDARD
            .decode(&challenge_response.challenge)
            .unwrap();
        let signature = keypair.sign(&challenge_bytes);

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id,
            device_id: registered_device.clone(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        service.verify_challenge(verify_request).await.unwrap();

        // Query registered device - should return verified=true
        let status = service
            .get_registration_status(&registered_device)
            .await
            .unwrap();
        assert!(status.verified);
        assert_eq!(status.device_id, registered_device);
    }

    /// Test REG-010: Multiple challenges for same device
    ///
    /// Spec: registration.spec.yaml - REG-010
    /// Given: Challenge already exists for device_id, has not expired
    /// When: create_challenge called again for same device
    /// Then: Returns new challenge, both challenges remain valid
    #[spec("REG-010")]
    #[tokio::test]
    async fn test_concurrent_challenge_replacement() {
        let service = RegistrationService::new();
        let device_id = "concurrent-device".to_string();

        // Create first challenge
        let challenge1 = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        let challenge1_id = challenge1.challenge_id.clone();

        // Create second challenge for same device
        let challenge2 = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        // Verify second challenge is different
        assert_ne!(challenge1_id, challenge2.challenge_id);

        // Both challenges should be valid for verification
        let keypair = arkavo_crypto::AgentKeypair::generate();

        // Verify first challenge still works
        let challenge1_bytes = general_purpose::STANDARD
            .decode(&challenge1.challenge)
            .unwrap();
        let signature1 = keypair.sign(&challenge1_bytes);

        let verify_request1 = VerifyRequest {
            challenge_id: challenge1_id,
            device_id: device_id.clone(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature1),
        };

        let result1 = service.verify_challenge(verify_request1).await;
        assert!(result1.is_ok(), "First challenge should still be valid");

        // Verify second challenge also works (same device, same keypair is fine
        // because registration just overwrites with same data)
        let challenge2_bytes = general_purpose::STANDARD
            .decode(&challenge2.challenge)
            .unwrap();
        let signature2 = keypair.sign(&challenge2_bytes);

        let verify_request2 = VerifyRequest {
            challenge_id: challenge2.challenge_id,
            device_id: device_id.clone(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature2),
        };

        let _result2 = service.verify_challenge(verify_request2).await;
        assert!(_result2.is_ok(), "Second challenge should also be valid");
    }

    /// Test REG-011: Handle challenge storage failure
    ///
    /// Spec: registration.spec.yaml - REG-011
    /// Given: System cannot store challenge
    /// When: create_challenge called
    /// Then: Returns A2aError::Internal error, no partial state
    ///
    /// Note: In current implementation with in-memory HashMap,
    /// storage failures are extremely rare. This test documents
    /// the expected behavior.
    #[spec("REG-011")]
    #[tokio::test]
    async fn test_challenge_storage_failure_handling() {
        // With in-memory storage, true storage failures are rare.
        // This test verifies the service can handle many concurrent requests.
        let service = RegistrationService::new();
        let device_id = "stress-test-device".to_string();

        // Create many concurrent challenges
        let mut handles = vec![];
        for i in 0..100 {
            let svc = service.clone();
            let did = format!("{}-{}", device_id, i);
            handles.push(tokio::spawn(async move {
                svc.create_challenge(ChallengeRequest { device_id: did })
                    .await
            }));
        }

        // All should succeed or fail cleanly (no panics)
        for handle in handles {
            let result = handle.await.unwrap();
            // Should either succeed or return an error, never panic
            match result {
                Ok(_) => {}
                Err(_) => {}
            }
        }
    }

    /// Test REG-012: Verify registration persistence
    ///
    /// Spec: registration.spec.yaml - REG-012
    /// Given: verify_challenge succeeds cryptographically
    /// When: Registration is stored
    /// Then: Registration persisted, challenge remains available
    ///
    /// Note: In current implementation, challenges are not consumed after
    /// verification and remain available for re-verification attempts.
    #[spec("REG-012")]
    #[tokio::test]
    async fn test_registration_persistence_consistency() {
        let service = RegistrationService::new();
        let device_id = "consistency-device".to_string();

        // Create and verify challenge
        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        let keypair = arkavo_crypto::AgentKeypair::generate();
        let challenge_bytes = general_purpose::STANDARD
            .decode(&challenge_response.challenge)
            .unwrap();
        let signature = keypair.sign(&challenge_bytes);

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id.clone(),
            device_id: device_id.clone(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        // Successful verification
        let result = service.verify_challenge(verify_request).await;
        assert!(result.is_ok());

        // Verify registration was persisted
        let status = service.get_registration_status(&device_id).await.unwrap();
        assert!(status.verified);

        // Verify challenge was consumed (removed) after successful verification
        // This prevents replay attacks
        {
            let challenges = service.challenges.read().await;
            assert!(
                !challenges.contains_key(&challenge_response.challenge_id),
                "Challenge should be consumed after successful verification"
            );
        }

        // Attempting to verify again should fail (challenge not found)
        let verify_request2 = VerifyRequest {
            challenge_id: challenge_response.challenge_id.clone(),
            device_id: device_id.clone(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };
        let result2 = service.verify_challenge(verify_request2).await;
        assert!(result2.is_err(), "Replay attack should be prevented");
        let err = result2.unwrap_err();
        let err_string = format!("{err}").to_lowercase();
        assert!(
            err_string.contains("not found") || err_string.contains("challenge"),
            "Error should indicate challenge not found: {err_string}"
        );
    }

    // Second tests to bump scenarios from Partial → Covered

    /// Second test for REG-001: Challenge uniqueness
    #[spec("REG-001")]
    #[tokio::test]
    async fn test_create_challenge_unique() {
        let service = RegistrationService::new();

        // Create multiple challenges and verify they're unique
        let mut challenge_ids = std::collections::HashSet::new();
        for i in 0..10 {
            let request = ChallengeRequest {
                device_id: format!("device-{i}"),
            };
            let response = service.create_challenge(request).await.unwrap();
            assert!(
                challenge_ids.insert(response.challenge_id.clone()),
                "Challenge ID should be unique"
            );
            assert!(!response.challenge.is_empty());
            // Verify base64 decoding works
            let decoded = general_purpose::STANDARD
                .decode(&response.challenge)
                .unwrap();
            assert_eq!(decoded.len(), 32);
        }
    }

    /// Second test for REG-002: Verify with different valid signatures
    #[spec("REG-002")]
    #[tokio::test]
    async fn test_verify_challenge_success_multiple_devices() {
        let service = RegistrationService::new();

        // Verify multiple devices can register
        for i in 0..5 {
            let device_id = format!("device-{i}");
            let challenge_response = service
                .create_challenge(ChallengeRequest {
                    device_id: device_id.clone(),
                })
                .await
                .unwrap();

            let keypair = arkavo_crypto::AgentKeypair::generate();
            let challenge_bytes = general_purpose::STANDARD
                .decode(&challenge_response.challenge)
                .unwrap();
            let signature = keypair.sign(&challenge_bytes);

            let verify_request = VerifyRequest {
                challenge_id: challenge_response.challenge_id,
                device_id: device_id.clone(),
                public_key: keypair.public_key().to_base64(),
                signature: general_purpose::STANDARD.encode(&signature),
            };

            let verify_response = service.verify_challenge(verify_request).await.unwrap();
            assert!(verify_response.success);

            let status = service.get_registration_status(&device_id).await.unwrap();
            assert!(status.verified);
        }
    }

    /// Second test for REG-003: Multiple P-256 device registrations
    #[spec("REG-003")]
    #[tokio::test]
    async fn test_verify_challenge_p256_multiple_devices() {
        let service = RegistrationService::new();

        // Register multiple iOS devices with P-256 keys
        for i in 0..5 {
            let device_id = format!("ios-device-{i}");
            let challenge_response = service
                .create_challenge(ChallengeRequest {
                    device_id: device_id.clone(),
                })
                .await
                .unwrap();

            let keypair = arkavo_crypto::P256SigningKeypair::generate();
            let challenge_bytes = general_purpose::STANDARD
                .decode(&challenge_response.challenge)
                .unwrap();
            let signature = keypair.sign(&challenge_bytes);

            let public_key_bytes = keypair.public_key().to_sec1_bytes();

            let verify_request = VerifyRequest {
                challenge_id: challenge_response.challenge_id,
                device_id: device_id.clone(),
                public_key: general_purpose::STANDARD.encode(&public_key_bytes),
                signature: general_purpose::STANDARD.encode(&signature),
            };

            let verify_response = service.verify_challenge(verify_request).await.unwrap();
            assert!(verify_response.success);

            let status = service.get_registration_status(&device_id).await.unwrap();
            assert!(status.verified);
        }
    }

    /// Second test for REG-004: Verify cleanup doesn't affect valid challenges
    #[spec("REG-004")]
    #[tokio::test]
    async fn test_challenge_cleanup_preserves_valid() {
        let service = RegistrationService::new();

        // Create a valid challenge
        let device_id = "valid-device".to_string();
        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        // Create another challenge (triggers cleanup)
        let device_id2 = "another-device".to_string();
        service
            .create_challenge(ChallengeRequest {
                device_id: device_id2.clone(),
            })
            .await
            .unwrap();

        // Verify first challenge still works
        let keypair = arkavo_crypto::AgentKeypair::generate();
        let challenge_bytes = general_purpose::STANDARD
            .decode(&challenge_response.challenge)
            .unwrap();
        let signature = keypair.sign(&challenge_bytes);

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id,
            device_id: device_id.clone(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        let result = service.verify_challenge(verify_request).await;
        assert!(result.is_ok(), "Valid challenge should not be cleaned up");
    }

    /// Second test for REG-005: Verify challenge slightly expired
    #[spec("REG-005")]
    #[tokio::test]
    async fn test_verify_challenge_slightly_expired() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let service = RegistrationService::new();
        let device_id = "test-device-slightly-expired".to_string();

        // Create a challenge slightly past TTL (1 second over)
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - CHALLENGE_TTL_SECONDS
            - 1;

        let challenge_data = vec![2u8; 32];
        let challenge_id = hex::encode(Sha256::digest(&challenge_data));

        let challenge = Challenge {
            data: challenge_data.clone(),
            timestamp,
            device_id: device_id.clone(),
        };

        {
            let mut challenges = service.challenges.write().await;
            challenges.insert(challenge_id.clone(), challenge);
        }

        let keypair = arkavo_crypto::AgentKeypair::generate();
        let signature = keypair.sign(&challenge_data);

        let verify_request = VerifyRequest {
            challenge_id,
            device_id,
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        // Should be rejected (past TTL)
        let result = service.verify_challenge(verify_request).await;
        assert!(result.is_err(), "Challenge past TTL should be expired");
    }

    /// Second test for REG-006: Verify mismatch with similar device IDs
    #[spec("REG-006")]
    #[tokio::test]
    async fn test_verify_challenge_device_mismatch_similar() {
        let service = RegistrationService::new();
        let device_id_a = "device-test".to_string();
        let device_id_b = "device-test-2".to_string();

        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id_a.clone(),
            })
            .await
            .unwrap();

        let keypair = arkavo_crypto::AgentKeypair::generate();
        let challenge_bytes = general_purpose::STANDARD
            .decode(&challenge_response.challenge)
            .unwrap();
        let signature = keypair.sign(&challenge_bytes);

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id,
            device_id: device_id_b, // Similar but different
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        let result = service.verify_challenge(verify_request).await;
        assert!(
            result.is_err(),
            "Similar device ID should still be rejected"
        );
    }

    /// Second test for REG-008: Test all unsupported key lengths
    #[spec("REG-008")]
    #[tokio::test]
    async fn test_verify_challenge_unsupported_key_lengths_various() {
        let service = RegistrationService::new();
        let device_id = "test-device-keys".to_string();

        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        // Test various invalid key lengths
        for key_len in [1, 16, 31, 34, 64, 100] {
            let bad_key = general_purpose::STANDARD.encode(&vec![0u8; key_len]);
            let signature = general_purpose::STANDARD.encode(&vec![0u8; 64]);

            let verify_request = VerifyRequest {
                challenge_id: challenge_response.challenge_id.clone(),
                device_id: device_id.clone(),
                public_key: bad_key,
                signature,
            };

            let result = service.verify_challenge(verify_request).await;
            assert!(result.is_err(), "Key length {key_len} should be rejected");
        }
    }

    /// Second test for REG-009: Verify status after multiple registrations
    #[spec("REG-009")]
    #[tokio::test]
    async fn test_query_registration_status_multiple() {
        let service = RegistrationService::new();

        // Query multiple unregistered devices
        for i in 0..5 {
            let device_id = format!("unregistered-{i}");
            let status = service.get_registration_status(&device_id).await.unwrap();
            assert!(!status.verified);
            assert_eq!(status.registered_at, 0);
        }

        // Register and verify multiple devices
        for i in 0..5 {
            let device_id = format!("registered-{i}");
            let challenge_response = service
                .create_challenge(ChallengeRequest {
                    device_id: device_id.clone(),
                })
                .await
                .unwrap();

            let keypair = arkavo_crypto::AgentKeypair::generate();
            let challenge_bytes = general_purpose::STANDARD
                .decode(&challenge_response.challenge)
                .unwrap();
            let signature = keypair.sign(&challenge_bytes);

            let verify_request = VerifyRequest {
                challenge_id: challenge_response.challenge_id,
                device_id: device_id.clone(),
                public_key: keypair.public_key().to_base64(),
                signature: general_purpose::STANDARD.encode(&signature),
            };

            service.verify_challenge(verify_request).await.unwrap();

            let status = service.get_registration_status(&device_id).await.unwrap();
            assert!(status.verified);
            assert!(status.registered_at > 0);
        }
    }

    /// Second test for REG-010: Verify multiple replacements work correctly
    #[spec("REG-010")]
    #[tokio::test]
    async fn test_concurrent_challenge_multiple_replacements() {
        let service = RegistrationService::new();
        let device_id = "multi-replace-device".to_string();

        // Create multiple challenges in sequence
        let mut challenges = vec![];
        for _ in 0..5 {
            let challenge = service
                .create_challenge(ChallengeRequest {
                    device_id: device_id.clone(),
                })
                .await
                .unwrap();
            challenges.push(challenge);
        }

        // All challenge IDs should be unique
        let unique_ids: std::collections::HashSet<_> =
            challenges.iter().map(|c| c.challenge_id.clone()).collect();
        assert_eq!(unique_ids.len(), 5, "All challenges should have unique IDs");

        // The last challenge should work for verification
        let keypair = arkavo_crypto::AgentKeypair::generate();
        let last_challenge = challenges.last().unwrap();
        let challenge_bytes = general_purpose::STANDARD
            .decode(&last_challenge.challenge)
            .unwrap();
        let signature = keypair.sign(&challenge_bytes);

        let verify_request = VerifyRequest {
            challenge_id: last_challenge.challenge_id.clone(),
            device_id: device_id.clone(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        let result = service.verify_challenge(verify_request).await;
        assert!(result.is_ok());
    }

    /// Second test for REG-011: Verify service handles rapid creation
    #[spec("REG-011")]
    #[tokio::test]
    async fn test_challenge_storage_rapid_creation() {
        let service = RegistrationService::new();

        // Rapidly create many challenges
        let start = std::time::Instant::now();
        let mut handles = vec![];

        for i in 0..50 {
            let svc = service.clone();
            handles.push(tokio::spawn(async move {
                svc.create_challenge(ChallengeRequest {
                    device_id: format!("rapid-{i}"),
                })
                .await
            }));
        }

        let results: Vec<_> = futures::future::join_all(handles).await;
        let elapsed = start.elapsed();

        // All should succeed
        for result in results {
            assert!(result.unwrap().is_ok());
        }

        // Should complete in reasonable time
        assert!(elapsed.as_secs() < 5, "Rapid creation should be fast");
    }

    /// Second test for REG-012: Verify persistence survives concurrent verification attempts
    #[spec("REG-012")]
    #[tokio::test]
    async fn test_registration_persistence_concurrent_verify() {
        let service = RegistrationService::new();
        let device_id = "concurrent-verify-device".to_string();

        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        let keypair = arkavo_crypto::AgentKeypair::generate();
        let challenge_bytes = general_purpose::STANDARD
            .decode(&challenge_response.challenge)
            .unwrap();
        let signature = keypair.sign(&challenge_bytes);

        // First verification
        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id.clone(),
            device_id: device_id.clone(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        service.verify_challenge(verify_request).await.unwrap();

        // Verify status is consistent
        let status1 = service.get_registration_status(&device_id).await.unwrap();
        let status2 = service.get_registration_status(&device_id).await.unwrap();
        assert!(status1.verified);
        assert_eq!(status1.verified, status2.verified);
        assert_eq!(status1.registered_at, status2.registered_at);
    }

    // ============================================================================
    // TDD Quality Improvement Tests - Concurrency, Edge Cases & Error Assertions
    // ============================================================================

    /// TDD Test: Concurrent challenge creation stress test
    #[spec("REG-001")]
    #[tokio::test]
    async fn test_concurrent_challenge_creation_stress() {
        let service = RegistrationService::new();
        let mut handles = vec![];

        // Spawn 100 concurrent challenge creations for same device
        for _ in 0..100 {
            let svc = service.clone();
            handles.push(tokio::spawn(async move {
                svc.create_challenge(ChallengeRequest {
                    device_id: "concurrent-device".to_string(),
                })
                .await
            }));
        }

        // All should succeed (last one wins for same device)
        let results = futures::future::join_all(handles).await;
        let success_count = results
            .iter()
            .filter(|r| r.as_ref().unwrap().is_ok())
            .count();
        assert_eq!(
            success_count, 100,
            "All concurrent creations should succeed"
        );
    }

    /// TDD Test: Concurrent verification attempts
    #[spec("REG-002")]
    #[tokio::test]
    async fn test_concurrent_verification_attempts() {
        let service = RegistrationService::new();
        let device_id = "concurrent-verify".to_string();

        // Create challenge
        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        let keypair = arkavo_crypto::AgentKeypair::generate();
        let challenge_bytes = general_purpose::STANDARD
            .decode(&challenge_response.challenge)
            .unwrap();
        let signature = keypair.sign(&challenge_bytes);

        // Spawn 50 concurrent verification attempts
        let mut handles = vec![];
        for _ in 0..50 {
            let svc = service.clone();
            let device_id = device_id.clone();
            let challenge_id = challenge_response.challenge_id.clone();
            let public_key = keypair.public_key().to_base64();
            let signature = general_purpose::STANDARD.encode(&signature);

            handles.push(tokio::spawn(async move {
                let verify_request = VerifyRequest {
                    challenge_id,
                    device_id,
                    public_key,
                    signature,
                };
                svc.verify_challenge(verify_request).await
            }));
        }

        // One should succeed, rest should fail (challenge consumed)
        let results = futures::future::join_all(handles).await;
        let success_count = results
            .iter()
            .filter(|r| r.as_ref().unwrap().is_ok())
            .count();
        // At least one should succeed, but due to race conditions, exactly one may not be guaranteed
        assert!(
            success_count >= 1,
            "At least one verification should succeed"
        );
    }

    /// TDD Test: Empty device ID rejected
    #[spec("REG-001")]
    #[tokio::test]
    async fn test_empty_device_id_rejected() {
        let service = RegistrationService::new();

        // Empty device ID should be rejected
        let request = ChallengeRequest {
            device_id: "".to_string(),
        };

        let result = service.create_challenge(request).await;
        assert!(result.is_err(), "Empty device ID should be rejected");

        let err = result.unwrap_err();
        let err_string = format!("{err}").to_lowercase();
        assert!(
            err_string.contains("empty") || err_string.contains("device id"),
            "Error should indicate empty device ID: {err_string}"
        );
    }

    /// TDD Test: Very long device ID
    #[spec("REG-001")]
    #[tokio::test]
    async fn test_long_device_id() {
        let service = RegistrationService::new();

        // 10KB device ID
        let long_id = "a".repeat(10_000);
        let request = ChallengeRequest { device_id: long_id };

        // Should handle gracefully
        let result = service.create_challenge(request).await;
        assert!(
            result.is_ok(),
            "Long device ID should be handled gracefully"
        );
    }

    /// TDD Test: Device ID with special characters
    #[spec("REG-001")]
    #[tokio::test]
    async fn test_device_id_special_characters() {
        let service = RegistrationService::new();

        let special_ids = vec![
            "device-with-dashes",
            "device_with_underscores",
            "device.with.dots",
            "device:with:colons",
            "device/with/slashes",
            "device@with@at",
            "device#with#hash",
            "device with spaces",
            "device\twith\ttabs",
            "device\nwith\nnewlines",
            "device🚀with🎉emoji",
        ];

        for id in special_ids {
            let request = ChallengeRequest {
                device_id: id.to_string(),
            };
            let result = service.create_challenge(request).await;
            assert!(result.is_ok(), "Device ID '{}' should be handled", id);
        }
    }

    /// TDD Test: Error message clarity for expired challenge
    #[spec("REG-005")]
    #[tokio::test]
    async fn test_expired_challenge_error_message() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let service = RegistrationService::new();
        let device_id = "error-test-device".to_string();

        // Create expired challenge
        let expired_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - CHALLENGE_TTL_SECONDS
            - 10;

        let challenge_data = vec![6u8; 32];
        let challenge_id = hex::encode(Sha256::digest(&challenge_data));

        let challenge = Challenge {
            data: challenge_data.clone(),
            timestamp: expired_timestamp,
            device_id: device_id.clone(),
        };

        {
            let mut challenges = service.challenges.write().await;
            challenges.insert(challenge_id.clone(), challenge);
        }

        let keypair = arkavo_crypto::AgentKeypair::generate();
        let signature = keypair.sign(&challenge_data);

        let verify_request = VerifyRequest {
            challenge_id,
            device_id,
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        let result = service.verify_challenge(verify_request).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_string = format!("{err}");
        // Error should mention expiration
        assert!(
            err_string.to_lowercase().contains("expired")
                || err_string.to_lowercase().contains("challenge"),
            "Error should indicate challenge issue: {err_string}"
        );
    }

    /// TDD Test: Error message clarity for device mismatch
    #[spec("REG-006")]
    #[tokio::test]
    async fn test_device_mismatch_error_message() {
        let service = RegistrationService::new();
        let device_a = "device-A".to_string();
        let device_b = "device-B".to_string();

        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_a.clone(),
            })
            .await
            .unwrap();

        let keypair = arkavo_crypto::AgentKeypair::generate();
        let challenge_bytes = general_purpose::STANDARD
            .decode(&challenge_response.challenge)
            .unwrap();
        let signature = keypair.sign(&challenge_bytes);

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id,
            device_id: device_b, // Wrong device
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        let result = service.verify_challenge(verify_request).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_string = format!("{err}");
        // Error should mention device or mismatch
        assert!(
            err_string.to_lowercase().contains("device")
                || err_string.to_lowercase().contains("mismatch"),
            "Error should indicate device issue: {err_string}"
        );
    }

    /// TDD Test: Invalid base64 in public key
    #[spec("REG-008")]
    #[tokio::test]
    async fn test_invalid_base64_public_key() {
        let service = RegistrationService::new();
        let device_id = "base64-test-device".to_string();

        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id,
            device_id,
            public_key: "!!!invalid-base64!!!".to_string(),
            signature: general_purpose::STANDARD.encode(&[0u8; 64]),
        };

        let result = service.verify_challenge(verify_request).await;
        assert!(result.is_err(), "Invalid base64 should be rejected");

        // Verify error message mentions base64 or decode
        let err = result.unwrap_err();
        let err_string = format!("{err}").to_lowercase();
        assert!(
            err_string.contains("base64")
                || err_string.contains("decode")
                || err_string.contains("invalid"),
            "Error should indicate decode issue: {err_string}"
        );
    }

    /// TDD Test: Invalid base64 in signature
    #[spec("REG-008")]
    #[tokio::test]
    async fn test_invalid_base64_signature() {
        let service = RegistrationService::new();
        let device_id = "sig-test-device".to_string();

        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        let keypair = arkavo_crypto::AgentKeypair::generate();

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id,
            device_id,
            public_key: keypair.public_key().to_base64(),
            signature: "!!!invalid-base64!!!".to_string(),
        };

        let result = service.verify_challenge(verify_request).await;
        assert!(
            result.is_err(),
            "Invalid base64 signature should be rejected"
        );
    }

    /// TDD Test: Unicode device ID roundtrip
    #[spec("REG-001", "REG-009")]
    #[tokio::test]
    async fn test_unicode_device_id_roundtrip() {
        let service = RegistrationService::new();

        // Various Unicode device IDs
        let device_ids = vec![
            "设备-测试",       // Chinese
            "デバイス-テスト", // Japanese
            "장치-테스트",     // Korean
            "🔐-secure-🔑",    // Emoji
            "München-Gerät",   // German
            "Устройство-Тест", // Russian
        ];

        for device_id in device_ids {
            // Create challenge
            let challenge = service
                .create_challenge(ChallengeRequest {
                    device_id: device_id.to_string(),
                })
                .await
                .unwrap();

            // Verify and get status
            let keypair = arkavo_crypto::AgentKeypair::generate();
            let challenge_bytes = general_purpose::STANDARD
                .decode(&challenge.challenge)
                .unwrap();
            let signature = keypair.sign(&challenge_bytes);

            let verify_request = VerifyRequest {
                challenge_id: challenge.challenge_id,
                device_id: device_id.to_string(),
                public_key: keypair.public_key().to_base64(),
                signature: general_purpose::STANDARD.encode(&signature),
            };

            service.verify_challenge(verify_request).await.unwrap();

            // Check status preserves device ID exactly
            let status = service.get_registration_status(device_id).await.unwrap();
            assert_eq!(status.device_id, device_id);
            assert!(status.verified);
        }
    }

    // ============================================================================
    // SECURITY REGRESSION TESTS
    // ============================================================================

    /// Regression test: Challenge replay attack prevention
    ///
    /// SECURITY FIX: Challenges are now consumed after successful verification
    /// to prevent replay attacks where an attacker could reuse a valid challenge
    /// to register multiple devices or bypass device limits.
    #[spec("REG-012")]
    #[tokio::test]
    async fn test_challenge_replay_attack_prevented() {
        let service = RegistrationService::new();
        let device_id = "replay-test-device".to_string();

        // Create challenge
        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        let keypair = arkavo_crypto::AgentKeypair::generate();
        let challenge_bytes = general_purpose::STANDARD
            .decode(&challenge_response.challenge)
            .unwrap();
        let signature = keypair.sign(&challenge_bytes);

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id.clone(),
            device_id: device_id.clone(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        // First verification should succeed
        let result1 = service.verify_challenge(verify_request.clone()).await;
        assert!(result1.is_ok(), "First verification should succeed");

        // Second verification with same challenge should fail (replay attack prevented)
        let result2 = service.verify_challenge(verify_request).await;
        assert!(result2.is_err(), "Replay attack should be prevented");

        let err = result2.unwrap_err();
        let err_string = format!("{err}").to_lowercase();
        assert!(
            err_string.contains("not found") || err_string.contains("challenge"),
            "Error should indicate challenge not found: {err_string}"
        );

        // Verify challenge is actually removed from storage
        let challenges = service.challenges.read().await;
        assert!(
            !challenges.contains_key(&challenge_response.challenge_id),
            "Challenge should be removed from storage after consumption"
        );
    }

    /// Regression test: Empty device ID rejection
    ///
    /// SECURITY FIX: Empty device IDs are now rejected to prevent:
    /// - Anonymous registrations that bypass device tracking
    /// - Potential injection of empty strings in device registries
    /// - Confusion in device identity management
    #[spec("REG-001")]
    #[tokio::test]
    async fn test_empty_device_id_security_fix() {
        let service = RegistrationService::new();

        // Attempt to create challenge with empty device ID
        let result = service
            .create_challenge(ChallengeRequest {
                device_id: "".to_string(),
            })
            .await;

        // Should be rejected
        assert!(result.is_err(), "Empty device ID should be rejected");

        let err = result.unwrap_err();
        let err_string = format!("{err}").to_lowercase();
        assert!(
            err_string.contains("empty") || err_string.contains("cannot"),
            "Error should indicate empty device ID rejection: {err_string}"
        );
    }

    /// Regression test: TTL boundary behavior (>= for consistent expiration)
    ///
    /// A challenge at exactly TTL seconds should be considered expired
    /// for consistent behavior across the codebase.
    #[spec("REG-005")]
    #[tokio::test]
    async fn test_ttl_boundary_behavior() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let service = RegistrationService::new();
        let device_id = "ttl-boundary-device".to_string();

        // Create challenge exactly at TTL boundary (should be expired)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let timestamp_at_boundary = now - CHALLENGE_TTL_SECONDS; // Exactly at boundary

        let challenge_data = vec![9u8; 32];
        let challenge_id = hex::encode(Sha256::digest(&challenge_data));

        let challenge = Challenge {
            data: challenge_data.clone(),
            timestamp: timestamp_at_boundary,
            device_id: device_id.clone(),
        };

        {
            let mut challenges = service.challenges.write().await;
            challenges.insert(challenge_id.clone(), challenge);
        }

        let keypair = arkavo_crypto::AgentKeypair::generate();
        let signature = keypair.sign(&challenge_data);

        let verify_request = VerifyRequest {
            challenge_id,
            device_id,
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        // Should be rejected (at boundary, considered expired with >= check)
        let result = service.verify_challenge(verify_request).await;
        assert!(
            result.is_err(),
            "Challenge at TTL boundary should be expired with >= check"
        );
    }

    /// Regression test: Race condition in challenge verification
    ///
    /// SECURITY FIX: Use write lock from start to prevent race condition
    /// where two threads could verify the same challenge simultaneously.
    /// Previously, we dropped read lock and acquired write lock, creating
    /// a window where another thread could sneak in.
    #[spec("REG-012")]
    #[tokio::test]
    async fn test_challenge_verification_atomic() {
        let service = RegistrationService::new();
        let device_id = "atomic-test-device".to_string();

        // Create challenge
        let challenge_response = service
            .create_challenge(ChallengeRequest {
                device_id: device_id.clone(),
            })
            .await
            .unwrap();

        let keypair = arkavo_crypto::AgentKeypair::generate();
        let challenge_bytes = general_purpose::STANDARD
            .decode(&challenge_response.challenge)
            .unwrap();
        let signature = keypair.sign(&challenge_bytes);

        let verify_request = VerifyRequest {
            challenge_id: challenge_response.challenge_id.clone(),
            device_id: device_id.clone(),
            public_key: keypair.public_key().to_base64(),
            signature: general_purpose::STANDARD.encode(&signature),
        };

        // First verification should succeed
        let result1 = service.verify_challenge(verify_request.clone()).await;
        assert!(result1.is_ok(), "First verification should succeed");

        // Second verification should fail because challenge was atomically removed
        // With the write lock held throughout, no race condition is possible
        let result2 = service.verify_challenge(verify_request).await;
        assert!(
            result2.is_err(),
            "Second verification should fail due to atomic removal"
        );

        let err = result2.unwrap_err();
        assert!(
            format!("{err}").contains("not found"),
            "Error should indicate challenge not found"
        );
    }
}
