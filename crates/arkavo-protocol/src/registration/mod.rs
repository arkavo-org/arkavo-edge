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

impl RegistrationService {
    pub fn new() -> Self {
        Self {
            challenges: Arc::new(RwLock::new(HashMap::new())),
            registrations: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn create_challenge(&self, request: ChallengeRequest) -> Result<ChallengeResponse> {
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

        self.cleanup_expired_challenges(&mut challenges, timestamp)
            .await;

        Ok(ChallengeResponse {
            challenge_id,
            challenge: general_purpose::STANDARD.encode(&challenge_data),
            timestamp,
        })
    }

    pub async fn verify_challenge(&self, request: VerifyRequest) -> Result<VerifyResponse> {
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let challenges = self.challenges.read().await;
        let challenge = challenges
            .get(&request.challenge_id)
            .ok_or_else(|| A2aError::InvalidRequest("Challenge not found".to_string()))?;

        if current_time - challenge.timestamp > CHALLENGE_TTL_SECONDS {
            return Err(A2aError::InvalidRequest("Challenge expired".to_string()));
        }

        if challenge.device_id != request.device_id {
            return Err(A2aError::InvalidRequest("Device ID mismatch".to_string()));
        }

        let public_key_bytes = general_purpose::STANDARD
            .decode(&request.public_key)
            .map_err(|e| A2aError::InvalidRequest(format!("Invalid public key: {}", e)))?;

        let signature_bytes = general_purpose::STANDARD
            .decode(&request.signature)
            .map_err(|e| A2aError::InvalidRequest(format!("Invalid signature: {}", e)))?;

        let public_key = arkavo_crypto::AgentPublicKey::from_bytes(&public_key_bytes)
            .map_err(|e| A2aError::InvalidRequest(format!("Invalid public key format: {}", e)))?;

        public_key
            .verify(&challenge.data, &signature_bytes)
            .map_err(|_| {
                A2aError::AuthenticationFailed("Signature verification failed".to_string())
            })?;

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

    async fn cleanup_expired_challenges(
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
}
